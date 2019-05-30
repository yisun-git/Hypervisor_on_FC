// Copyright 2018 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Portions Copyright 2017 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the THIRD-PARTY file.

use std::io;
use std::os::unix::io::AsRawFd;
use std::result;
use std::sync::{Arc, Barrier};

use super::{HypContext, TimestampUs};
use arch;
#[cfg(target_arch = "x86_64")]
use cpuid::{c3, filter_cpuid, t2};
use default_syscalls;
use hypervisor::*;
use hypervisor::vcpu::*;
use hypervisor::x86_64::*;
use logger::{LogOption, Metric, LOGGER, METRICS};
use memory_model::{GuestAddress, GuestMemory, GuestMemoryError};
use sys_util::EventFd;
#[cfg(target_arch = "x86_64")]
use vmm_config::machine_config::CpuFeaturesTemplate;
use vmm_config::machine_config::VmConfig;

// TODO: using KVM value now. How about other hypervisor?
const MEM_LOG_DIRTY_PAGES: u32 = 0x1;

const MAGIC_IOPORT_SIGNAL_GUEST_BOOT_COMPLETE: u16 = 0x03f0;
const MAGIC_VALUE_SIGNAL_GUEST_BOOT_COMPLETE: u8 = 123;

/// Errors associated with the wrappers over KVM ioctls.
#[derive(Debug)]
pub enum Error {
    #[cfg(target_arch = "x86_64")]
    /// A call to cpuid instruction failed.
    CpuId(cpuid::Error),
    /// Invalid guest memory configuration.
    GuestMemory(GuestMemoryError),
    /// Hyperthreading flag is not initialized.
    HTNotInitialized,
    /// vCPU count is not initialized.
    VcpuCountNotInitialized,
    /// Cannot open the VM file descriptor.
    Vm(io::Error),
    /// Cannot open the VCPU file descriptor.
    Vcpu(io::Error),
    /// Cannot configure the microvm.
    VmSetup(io::Error),
    /// Cannot run the VCPUs.
    VcpuRun(io::Error),
    /// The call to KVM_SET_CPUID2 failed.
    SetSupportedCpusFailed(io::Error),
    /// The number of configured slots is bigger than the maximum reported by KVM.
    NotEnoughMemorySlots,
    #[cfg(target_arch = "x86_64")]
    /// Cannot set the local interruption due to bad configuration.
    LocalIntConfiguration(arch::x86_64::interrupts::Error),
    /// Cannot set the memory regions.
    SetUserMemoryRegion(io::Error),
    #[cfg(target_arch = "x86_64")]
    /// Error configuring the MSR registers
    MSRSConfiguration(arch::x86_64::regs::Error),
    #[cfg(target_arch = "aarch64")]
    /// Error configuring the general purpose aarch64 registers.
    REGSConfiguration(arch::aarch64::regs::Error),
    #[cfg(target_arch = "x86_64")]
    /// Error configuring the general purpose registers
    REGSConfiguration(arch::x86_64::regs::Error),
    #[cfg(target_arch = "x86_64")]
    /// Error configuring the special registers
    SREGSConfiguration(arch::x86_64::regs::Error),
    #[cfg(target_arch = "x86_64")]
    /// Error configuring the floating point related registers
    FPUConfiguration(arch::x86_64::regs::Error),
    /// Cannot configure the IRQ.
    Irq(io::Error),
    /// Cannot spawn a new vCPU thread.
    VcpuSpawn(io::Error),
    /// Unexpected KVM_RUN exit reason
    VcpuUnhandledKvmExit,
    #[cfg(target_arch = "aarch64")]
    /// Error setting up the global interrupt controller.
    SetupGIC(arch::aarch64::gic::Error),
    #[cfg(target_arch = "aarch64")]
    /// Error getting the Vcpu preferred target on Arm.
    VcpuArmPreferredTarget(io::Error),
    #[cfg(target_arch = "aarch64")]
    /// Error doing Vcpu Init on Arm.
    VcpuArmInit(io::Error),
}
pub type Result<T> = result::Result<T, Error>;

/// A wrapper around creating and using a VM.
pub struct GuestVm {
    fd: Box<Vm>,
    guest_mem: Option<GuestMemory>,

    // X86 specific fields.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    supported_cpuid: CpuId,

    // Arm specific fields.
    // On aarch64 we need to keep around the fd obtained by creating the VGIC device.
    #[cfg(target_arch = "aarch64")]
    irqchip_handle: Option<DeviceFd>,
}

impl GuestVm {
    /// Constructs a new `Vm` using the given `Hypervisor` instance.
    pub fn new(hyp: &Hypervisor) -> Result<Self> {
        //create fd for interacting with vm specific functions
        let vm_fd = hyp.create_vm().map_err(Error::Vm)?;
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        let cpuid = hyp
            .get_supported_cpuid(MAX_CPUID_ENTRIES)
            .map_err(Error::Vm)?;
        Ok(GuestVm {
            fd: vm_fd,
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            supported_cpuid: cpuid,
            guest_mem: None,
            #[cfg(target_arch = "aarch64")]
            irqchip_handle: None,
        })
    }

    /// Returns a clone of the supported `CpuId` for this Vm.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_supported_cpuid(&self) -> CpuId {
        self.supported_cpuid.clone()
    }

    /// Initializes the guest memory.
    pub fn memory_init(&mut self, guest_mem: GuestMemory, hyp_context: &HypContext) -> Result<()> {
        if guest_mem.num_regions() > hyp_context.max_memslots() {
            return Err(Error::NotEnoughMemorySlots);
        }
        guest_mem
            .with_regions(|index, guest_addr, size, host_addr| {
                info!("Guest memory starts at {:x?}", host_addr);

                let flags = if LOGGER.flags() & LogOption::LogDirtyPages as usize > 0 {
                    MEM_LOG_DIRTY_PAGES
                } else {
                    0
                };

                let slot = index as u32;
                let guest_phys_addr = guest_addr.offset() as u64;
                let memory_size = size as u64;
                let userspace_addr = host_addr as u64;
                self.fd.set_user_memory_region(slot,
                                               guest_phys_addr,
                                               memory_size,
                                               userspace_addr,
                                               flags)
            })
            .map_err(Error::SetUserMemoryRegion)?;
        self.guest_mem = Some(guest_mem);

        #[cfg(target_arch = "x86_64")]
        self.fd
            .set_tss_address(GuestAddress(arch::x86_64::layout::TSS_ADDRESS).offset())
            .map_err(Error::VmSetup)?;

        Ok(())
    }

    /// This function creates the irq chip and adds 3 interrupt events to the IRQ.
    #[cfg(target_arch = "x86_64")]
    pub fn setup_irqchip(
        &self,
        com_evt_1_3: &EventFd,
        com_evt_2_4: &EventFd,
        kbd_evt: &EventFd,
    ) -> Result<()> {
        self.fd.create_irq_chip().map_err(Error::VmSetup)?;

        self.fd
            .register_irqfd(com_evt_1_3.as_raw_fd(), 4)
            .map_err(Error::Irq)?;
        self.fd
            .register_irqfd(com_evt_2_4.as_raw_fd(), 3)
            .map_err(Error::Irq)?;
        self.fd
            .register_irqfd(kbd_evt.as_raw_fd(), 1)
            .map_err(Error::Irq)?;

        Ok(())
    }

    /// This function creates the GIC (Global Interrupt Controller).
    #[cfg(target_arch = "aarch64")]
    pub fn setup_irqchip(&mut self, vcpu_count: u8) -> Result<()> {
        self.irqchip_handle =
            Some(arch::aarch64::gic::create_gicv3(&self.fd, vcpu_count).map_err(Error::SetupGIC)?);
        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    /// Creates an in-kernel device model for the PIT.
    pub fn create_pit(&self) -> Result<()> {
        let mut pit_config = PitConfig::default();
        // We need to enable the emulation of a dummy speaker port stub so that writing to port 0x61
        // (i.e. KVM_SPEAKER_BASE_ADDRESS) does not trigger an exit to user space.
        pit_config.flags = PIT_SPEAKER_DUMMY;
        self.fd.create_pit2(pit_config).map_err(Error::VmSetup)?;
        Ok(())
    }

    /// Gets a reference to the guest memory owned by this VM.
    ///
    /// Note that `GuestMemory` does not include any device memory that may have been added after
    /// this VM was constructed.
    pub fn get_memory(&self) -> Option<&GuestMemory> {
        self.guest_mem.as_ref()
    }

    /// Gets a reference to the file descriptor owned by this VM.
    ///
    pub fn get_fd(&self) -> &Vm {
        &(*self.fd)
    }
}

/// A wrapper around creating and using a VCPU.
pub struct GuestVcpu {
    #[cfg(target_arch = "x86_64")]
    cpuid: CpuId,
    fd: Box<Vcpu + Send + 'static>,
    id: u8,
    io_bus: devices::Bus,
    mmio_bus: devices::Bus,
    create_ts: TimestampUs,
}

impl GuestVcpu {
    /// Constructs a new VCPU for `vm`.
    ///
    /// # Arguments
    ///
    /// * `id` - Represents the CPU number between [0, max vcpus).
    /// * `vm` - The virtual machine this vcpu will get attached to.
    pub fn new(
        id: u8,
        vm: &GuestVm,
        io_bus: devices::Bus,
        mmio_bus: devices::Bus,
        create_ts: TimestampUs,
    ) -> Result<Self> {
        let vcpu = vm.fd.create_vcpu(id).map_err(Error::Vcpu)?;

        // Initially the cpuid per vCPU is the one supported by this VM.
        Ok(GuestVcpu {
            #[cfg(target_arch = "x86_64")]
            cpuid: vm.get_supported_cpuid(),
            fd: vcpu,
            id,
            io_bus,
            mmio_bus,
            create_ts,
        })
    }

    #[cfg(target_arch = "x86_64")]
    /// Configures a x86_64 specific vcpu and should be called once per vcpu from the vcpu's thread.
    ///
    /// # Arguments
    ///
    /// * `machine_config` - Specifies necessary info used for the CPUID configuration.
    /// * `kernel_start_addr` - Offset from `guest_mem` at which the kernel starts.
    /// * `vm` - The virtual machine this vcpu will get attached to.
    pub fn configure(
        &mut self,
        machine_config: &VmConfig,
        kernel_start_addr: GuestAddress,
        vm: &GuestVm,
    ) -> Result<()> {
        // the MachineConfiguration has defaults for ht_enabled and vcpu_count hence it is safe to unwrap
        filter_cpuid(
            self.id,
            machine_config
                .vcpu_count
                .ok_or(Error::VcpuCountNotInitialized)?,
            machine_config.ht_enabled.ok_or(Error::HTNotInitialized)?,
            &mut self.cpuid,
        )
        .map_err(Error::CpuId)?;

        if let Some(template) = machine_config.cpu_template {
            match template {
                CpuFeaturesTemplate::T2 => t2::set_cpuid_entries(self.cpuid.mut_entries_slice()),
                CpuFeaturesTemplate::C3 => c3::set_cpuid_entries(self.cpuid.mut_entries_slice()),
            }
        }

        self.fd
            .set_cpuid2(&self.cpuid)
            .map_err(Error::SetSupportedCpusFailed)?;

        arch::x86_64::regs::setup_msrs(&(*self.fd)).map_err(Error::MSRSConfiguration)?;
        // Safe to unwrap because this method is called after the VM is configured
        let vm_memory = vm
            .get_memory()
            .ok_or(Error::GuestMemory(GuestMemoryError::MemoryNotInitialized))?;
        arch::x86_64::regs::setup_regs(&(*self.fd), kernel_start_addr.offset() as u64)
            .map_err(Error::REGSConfiguration)?;
        arch::x86_64::regs::setup_fpu(&(*self.fd)).map_err(Error::FPUConfiguration)?;
        arch::x86_64::regs::setup_sregs(vm_memory, &(*self.fd)).map_err(Error::SREGSConfiguration)?;
        arch::x86_64::interrupts::set_lint(&(*self.fd)).map_err(Error::LocalIntConfiguration)?;
        Ok(())
    }

    #[cfg(target_arch = "aarch64")]
    /// Configures an aarch64 specific vcpu.
    ///
    /// # Arguments
    ///
    /// * `_machine_config` - Specifies necessary info used for the CPUID configuration.
    /// * `kernel_load_addr` - Offset from `guest_mem` at which the kernel is loaded.
    /// * `vm` - The virtual machine this vcpu will get attached to.
    pub fn configure(
        &mut self,
        _machine_config: &VmConfig,
        kernel_load_addr: GuestAddress,
        vm: &GuestVm,
    ) -> Result<()> {
        let vm_memory = vm
            .get_memory()
            .ok_or(Error::GuestMemory(GuestMemoryError::MemoryNotInitialized))?;

        let mut vi: hypervisor::vm::VcpuInit = hypervisor::vm::VcpuInit::default();

        // This reads back the kernel's preferred target type.
        vm.fd
            .get_preferred_target(&mut vi)
            .map_err(Error::VcpuArmPreferredTarget)?;
        // We already checked that the capability is supported.
        vi.features[0] |= 1 << hypervisor::vm::ARM_VCPU_PSCI_0_2;
        // Non-boot cpus are powered off initially.
        if self.id > 0 {
            vi.features[0] |= 1 << hypervisor::vm::ARM_VCPU_POWER_OFF;
        }

        self.fd.vcpu_init(&vi).map_err(Error::VcpuArmInit)?;
        arch::aarch64::regs::setup_regs(&self.fd, self.id, kernel_load_addr.offset(), vm_memory)
            .map_err(Error::REGSConfiguration)?;
        Ok(())
    }

    fn run_emulation(&mut self) -> Result<()> {
        match self.fd.run() {
            Ok(run) => match run {
                VcpuExit::IoIn(addr, data) => {
                    self.io_bus.read(u64::from(addr), data);
                    METRICS.vcpu.exit_io_in.inc();
                    Ok(())
                }
                VcpuExit::IoOut(addr, data) => {
                    if addr == MAGIC_IOPORT_SIGNAL_GUEST_BOOT_COMPLETE
                        && data[0] == MAGIC_VALUE_SIGNAL_GUEST_BOOT_COMPLETE
                    {
                        super::Vmm::log_boot_time(&self.create_ts);
                    }
                    self.io_bus.write(u64::from(addr), data);
                    METRICS.vcpu.exit_io_out.inc();
                    Ok(())
                }
                VcpuExit::MmioRead(addr, data) => {
                    self.mmio_bus.read(addr, data);
                    METRICS.vcpu.exit_mmio_read.inc();
                    Ok(())
                }
                VcpuExit::MmioWrite(addr, data) => {
                    self.mmio_bus.write(addr, data);
                    METRICS.vcpu.exit_mmio_write.inc();
                    Ok(())
                }
                VcpuExit::Hlt => {
                    info!("Received KVM_EXIT_HLT signal");
                    Err(Error::VcpuUnhandledKvmExit)
                }
                VcpuExit::Shutdown => {
                    info!("Received KVM_EXIT_SHUTDOWN signal");
                    Err(Error::VcpuUnhandledKvmExit)
                }
                // Documentation specifies that below hypervisor exits are considered
                // errors.
                VcpuExit::FailEntry => {
                    METRICS.vcpu.failures.inc();
                    error!("Received KVM_EXIT_FAIL_ENTRY signal");
                    Err(Error::VcpuUnhandledKvmExit)
                }
                VcpuExit::InternalError => {
                    METRICS.vcpu.failures.inc();
                    error!("Received KVM_EXIT_INTERNAL_ERROR signal");
                    Err(Error::VcpuUnhandledKvmExit)
                }
                r => {
                    METRICS.vcpu.failures.inc();
                    // TODO: Are we sure we want to finish running a vcpu upon
                    // receiving a vm exit that is not necessarily an error?
                    error!("Unexpected exit reason on vcpu run: {:?}", r);
                    Err(Error::VcpuUnhandledKvmExit)
                }
            },
            // The unwrap on raw_os_error can only fail if we have a logic
            // error in our code in which case it is better to panic.
            Err(ref e) => {
                match e.raw_os_error().unwrap() {
                    // Why do we check for these if we only return EINVAL?
                    libc::EAGAIN | libc::EINTR => Ok(()),
                    _ => {
                        METRICS.vcpu.failures.inc();
                        error!("Failure during vcpu run: {}", e);
                        Err(Error::VcpuUnhandledKvmExit)
                    }
                }
            }
        }
    }

    /// Main loop of the vCPU thread.
    ///
    ///
    /// Runs the vCPU in KVM context in a loop. Handles KVM_EXITs then goes back in.
    /// Also registers a signal handler to be able to kick this thread out of KVM_RUN.
    /// Note that the state of the VCPU and associated VM must be setup first for this to do
    /// anything useful.
    pub fn run(
        &mut self,
        thread_barrier: Arc<Barrier>,
        seccomp_level: u32,
        vcpu_exit_evt: EventFd,
    ) {
        // Load seccomp filters for this vCPU thread.
        // Execution panics if filters cannot be loaded, use --seccomp-level=0 if skipping filters
        // altogether is the desired behaviour.
        if let Err(e) = default_syscalls::set_seccomp_level(seccomp_level) {
            panic!(
                "Failed to set the requested seccomp filters on vCPU {}: Error: {}",
                self.id, e
            );
        }

        thread_barrier.wait();

        while self.run_emulation().is_ok() {}

        // Nothing we need do for the success case.
        if let Err(e) = vcpu_exit_evt.write(1) {
            METRICS.vcpu.failures.inc();
            error!("Failed signaling vcpu exit event: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::super::devices;
    use super::*;

    use libc::{c_int, c_void, siginfo_t};
    use sys_util::{register_signal_handler, Killable, SignalHandler};

    // Auxiliary function being used throughout the tests.
    fn setup_vcpu() -> (GuestVm, GuestVcpu) {
        let hyp = HypContext::new(0).unwrap();
        let gm = GuestMemory::new(&[(GuestAddress(0), 0x10000)]).unwrap();
        let mut vm = GuestVm::new(hyp.fd()).expect("Cannot create new vm");
        assert!(vm.memory_init(gm, &hyp).is_ok());

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            let dummy_eventfd_1 = EventFd::new().unwrap();
            let dummy_eventfd_2 = EventFd::new().unwrap();
            let dummy_kbd_eventfd = EventFd::new().unwrap();

            vm.setup_irqchip(&dummy_eventfd_1, &dummy_eventfd_2, &dummy_kbd_eventfd)
                .unwrap();
            vm.create_pit().unwrap();
        }
        let vcpu = GuestVcpu::new(
            1,
            &vm,
            devices::Bus::new(),
            devices::Bus::new(),
            super::super::TimestampUs::default(),
        )
        .unwrap();
        #[cfg(target_arch = "aarch64")]
        {
            vm.setup_irqchip(1).expect("Cannot setup irqchip");
        }

        (vm, vcpu)
    }

    #[test]
    fn test_create_vm() {
        let hyp = HypContext::new(0).unwrap();
        let vm = GuestVm::new(hyp.fd()).expect("Cannot create new vm");

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            let mut cpuid = hyp 
                .hyp
                .get_supported_cpuid(MAX_CPUID_ENTRIES)
                .expect("Cannot get supported cpuid");
            assert_eq!(
                vm.get_supported_cpuid().mut_entries_slice(),
                cpuid.mut_entries_slice()
            );
        }
    }

    #[test]
    fn test_vm_memory_init_success() {
        let hyp = HypContext::new(0).unwrap();
        let gm = GuestMemory::new(&[(GuestAddress(0), 0x1000)]).unwrap();
        let mut vm = GuestVm::new(hyp.fd()).expect("Cannot create new vm");
        assert!(vm.memory_init(gm, &hyp).is_ok());
        let obj_addr = GuestAddress(0xf0);
        vm.get_memory()
            .unwrap()
            .write_obj_at_addr(67u8, obj_addr)
            .unwrap();
        let read_val: u8 = vm
            .get_memory()
            .unwrap()
            .read_obj_from_addr(obj_addr)
            .unwrap();
        assert_eq!(read_val, 67u8);
    }

    #[test]
    fn test_vm_memory_init_failure() {
        let hyp = HypContext::new(1).unwrap();
        let mut vm = GuestVm::new(hyp.fd()).expect("new vm failed");

        let start_addr1 = GuestAddress(0x0);
        let start_addr2 = GuestAddress(0x1000);
        let gm = GuestMemory::new(&[(start_addr1, 0x1000), (start_addr2, 0x1000)]).unwrap();

        assert!(vm.memory_init(gm, &hyp).is_err());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_setup_irqchip() {
        let hyp = HypContext::new(0).unwrap();
        let vm = GuestVm::new(hyp.fd()).expect("Cannot create new vm");
        let dummy_eventfd_1 = EventFd::new().unwrap();
        let dummy_eventfd_2 = EventFd::new().unwrap();
        let dummy_kbd_eventfd = EventFd::new().unwrap();

        vm.setup_irqchip(&dummy_eventfd_1, &dummy_eventfd_2, &dummy_kbd_eventfd)
            .expect("Cannot setup irqchip");
        let _vcpu = GuestVcpu::new(
            1,
            &vm,
            devices::Bus::new(),
            devices::Bus::new(),
            super::super::TimestampUs::default(),
        )
        .unwrap();
        // Trying to setup two irqchips will result in EEXIST error.
        assert!(vm
            .setup_irqchip(&dummy_eventfd_1, &dummy_eventfd_2, &dummy_kbd_eventfd)
            .is_err());
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn test_setup_irqchip() {
        let hyp = HypContext::new(0).unwrap();

        let mut vm = GuestVm::new(hyp.fd()).expect("Cannot create new vm");
        let vcpu_count = 1;
        let _vcpu = GuestVcpu::new(
            1,
            &vm,
            devices::Bus::new(),
            devices::Bus::new(),
            super::super::TimestampUs::default(),
        )
        .unwrap();

        vm.setup_irqchip(vcpu_count).expect("Cannot setup irqchip");
        // Trying to setup two irqchips will result in EEXIST error.
        assert!(vm.setup_irqchip(vcpu_count).is_err());
    }

    #[test]
    fn test_setup_irqchip_failure() {
        let hyp = HypContext::new(0).unwrap();
        // On aarch64, this needs to be mutable.
        #[allow(unused_mut)]
        let mut vm = GuestVm::new(hyp.fd()).expect("Cannot create new vm");
        let _vcpu = GuestVcpu::new(
            1,
            &vm,
            devices::Bus::new(),
            devices::Bus::new(),
            super::super::TimestampUs::default(),
        )
        .unwrap();

        #[cfg(target_arch = "x86_64")]
        {
            let dummy_eventfd_1 = EventFd::new().unwrap();
            let dummy_eventfd_2 = EventFd::new().unwrap();
            let dummy_kbd_eventfd = EventFd::new().unwrap();
            // Trying to setup irqchip after KVM_VCPU_CREATE was called will result in error on x86_64.
            assert!(vm
                .setup_irqchip(&dummy_eventfd_1, &dummy_eventfd_2, &dummy_kbd_eventfd)
                .is_err());
        }
        #[cfg(target_arch = "aarch64")]
        {
            // Trying to setup irqchip after KVM_VCPU_CREATE is actually the way to go on aarch64.
            assert!(vm.setup_irqchip(1).is_ok());
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_create_pit() {
        let hyp = HypContext::new(0).unwrap();
        let vm = GuestVm::new(hyp.fd()).expect("Cannot create new vm");

        assert!(vm.create_pit().is_ok());
        // Trying to setup two PITs will result in EEXIST error.
        assert!(vm.create_pit().is_err());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_configure_vcpu() {
        let (vm, mut vcpu) = setup_vcpu();

        let vm_config = VmConfig::default();
        assert!(vcpu.configure(&vm_config, GuestAddress(0), &vm).is_ok());

        // Test configure while using the T2 template.
        let mut vm_config = VmConfig::default();
        vm_config.cpu_template = Some(CpuFeaturesTemplate::T2);
        assert!(vcpu.configure(&vm_config, GuestAddress(0), &vm).is_ok());

        // Test configure while using the C3 template.
        let mut vm_config = VmConfig::default();
        vm_config.cpu_template = Some(CpuFeaturesTemplate::C3);
        assert!(vcpu.configure(&vm_config, GuestAddress(0), &vm).is_ok());
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn test_configure_vcpu() {
        let hyp = HypContext::new(0).unwrap();
        let gm = GuestMemory::new(&[(GuestAddress(0), 0x10000)]).unwrap();
        let mut vm = GuestVm::new(hyp.fd()).expect("new vm failed");
        assert!(vm.memory_init(gm, &hyp).is_ok());

        // Try it for when vcpu id is 0.
        let mut vcpu = GuestVcpu::new(
            0,
            &vm,
            devices::Bus::new(),
            devices::Bus::new(),
            super::super::TimestampUs::default(),
        )
        .unwrap();

        let vm_config = VmConfig::default();
        assert!(vcpu.configure(&vm_config, GuestAddress(0), &vm).is_ok());

        // Try it for when vcpu id is NOT 0.
        let mut vcpu = GuestVcpu::new(
            1,
            &vm,
            devices::Bus::new(),
            devices::Bus::new(),
            super::super::TimestampUs::default(),
        )
        .unwrap();

        assert!(vcpu.configure(&vm_config, GuestAddress(0), &vm).is_ok());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_run_vcpu() {
        extern "C" fn handle_signal(_: c_int, _: *mut siginfo_t, _: *mut c_void) {}

        let signum = 0;
        // We install a signal handler for the specified signal; otherwise the whole process will
        // be brought down when the signal is received, as part of the default behaviour. Signal
        // handlers are global, so we install this before starting the thread.
        unsafe {
            register_signal_handler(signum, SignalHandler::Siginfo(handle_signal), true)
                .expect("failed to register vcpu signal handler");
        }

        let (vm, mut vcpu) = setup_vcpu();

        let vm_config = VmConfig::default();
        #[cfg(target_arch = "x86_64")]
        assert!(vcpu.configure(&vm_config, GuestAddress(0), &vm).is_ok());

        let thread_barrier = Arc::new(Barrier::new(2));
        let exit_evt = EventFd::new().unwrap();

        let vcpu_thread_barrier = thread_barrier.clone();
        let vcpu_exit_evt = exit_evt.try_clone().expect("eventfd clone failed");
        let seccomp_level = 0;

        let thread = thread::Builder::new()
            .name("fc_vcpu0".to_string())
            .spawn(move || {
                vcpu.run(vcpu_thread_barrier, seccomp_level, vcpu_exit_evt);
            })
            .expect("failed to spawn thread ");

        thread_barrier.wait();

        // Wait to make sure the vcpu starts its KVM_RUN ioctl.
        thread::sleep(Duration::from_millis(100));

        // Kick the vcpu out of KVM_RUN.
        thread.kill(signum).expect("failed to signal thread");

        // Wait some more.
        thread::sleep(Duration::from_millis(100));

        // Validate vcpu handled the EINTR gracefully and didn't exit.
        let err = exit_evt.read().unwrap_err();
        assert_eq!(err.raw_os_error().unwrap(), libc::EAGAIN);
    }

    #[test]
    fn not_enough_mem_slots() {
        let hyp = HypContext::new(1).unwrap();
        let mut vm = GuestVm::new(hyp.fd()).expect("new vm failed");

        let start_addr1 = GuestAddress(0x0);
        let start_addr2 = GuestAddress(0x1000);
        let gm = GuestMemory::new(&[(start_addr1, 0x1000), (start_addr2, 0x1000)]).unwrap();

        assert!(vm.memory_init(gm, &hyp).is_err());
    }
}
