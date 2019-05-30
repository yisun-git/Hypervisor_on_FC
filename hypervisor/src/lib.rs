// Copyright 2018-2019 Intel Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may
// not use this file except in compliance with the License. You may obtain
// a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
// License for the specific language governing permissions and limitations
// under the License.

extern crate kvm_bindings;

pub mod vm;
pub mod vcpu;

//#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub mod x86_64;

//#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
//mod arm;

use std::{io, result};
use std::boxed::Box;

pub use crate::vm::Vm;

pub use crate::x86_64::CpuId;

/// A capability the hypervisor's interface can possibly expose.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
// We are allowing docs to be missing here because this enum is a wrapper
// over auto-generated code.
#[allow(missing_docs)]
pub enum Cap {
    Irqchip,
    Hlt,
    MmuShadowCacheControl,
    UserMemory,
    SetTssAddr,
    Vapic,
    ExtCpuid,
    Clocksource,
    NrVcpus,
    NrMemslots,
    Pit,
    NopIoDelay,
    PvMmu,
    MpState,
    CoalescedMmio,
    SyncMmu,
    Iommu,
    DestroyMemoryRegionWorks,
    UserNmi,
    SetGuestDebug,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    ReinjectControl,
    IrqRouting,
    IrqInjectStatus,
    AssignDevIrq,
    JoinMemoryRegionsWorks,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    Mce,
    Irqfd,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    Pit2,
    SetBootCpuId,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    PitState2,
    Ioeventfd,
    SetIdentityMapAddr,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    XenHvm,
    AdjustClock,
    InternalErrorData,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    VcpuEvents,
    S390Psw,
    PpcSegstate,
    Hyperv,
    HypervVapic,
    HypervSpin,
    PciSegment,
    PpcPairedSingles,
    IntrShadow,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    Debugregs,
    X86RobustSinglestep,
    PpcOsi,
    PpcUnsetIrq,
    EnableCap,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    Xsave,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    Xcrs,
    PpcGetPvinfo,
    PpcIrqLevel,
    AsyncPf,
    TscControl,
    GetTscKhz,
    PpcBookeSregs,
    SpaprTce,
    PpcSmt,
    PpcRma,
    MaxVcpus,
    PpcHior,
    PpcPapr,
    SwTlb,
    OneReg,
    S390Gmap,
    TscDeadlineTimer,
    S390Ucontrol,
    SyncRegs,
    Pci23,
    KvmclockCtrl,
    SignalMsi,
    PpcGetSmmuInfo,
    S390Cow,
    PpcAllocHtab,
    ReadonlyMem,
    IrqfdResample,
    PpcBookeWatchdog,
    PpcHtabFd,
    S390CssSupport,
    PpcEpr,
    ArmPsci,
    ArmSetDeviceAddr,
    DeviceCtrl,
    IrqMpic,
    PpcRtas,
    IrqXics,
    ArmEl132bit,
    SpaprMultitce,
    ExtEmulCpuid,
    HypervTime,
    IoapicPolarityIgnored,
    EnableCapVm,
    S390Irqchip,
    IoeventfdNoLength,
    VmAttributes,
    ArmPsci02,
    PpcFixupHcall,
    PpcEnableHcall,
    CheckExtensionVm,
    S390UserSigp,
    ImmediateExit,
}

pub type Result<T> = result::Result<T, io::Error>;

pub trait Hypervisor {
    fn create_vm(&self) -> Result<Box<Vm>>;
    fn get_api_version(&self) -> i32;
    fn check_extension(&self, c: Cap) -> bool;
    fn get_vcpu_mmap_size(&self) -> Result<usize>;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn get_emulated_cpuid(&self, max_entries_count: usize) -> Result<CpuId>;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn get_supported_cpuid(&self, max_entries_count: usize) -> Result<CpuId>;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn get_msr_index_list(&self) -> Result<Vec<u32>>;
}
