// Copyright 2018 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Portions Copyright 2017 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the THIRD-PARTY file.

use kvm_bindings::*;

extern crate hypervisor;

use hypervisor::Cap;

// The kvm API to convert Cap to KVM value.
#[allow(missing_docs)]
pub fn cap_conv(cap: Cap) -> u32 {
    match cap {
        Cap::Irqchip => KVM_CAP_IRQCHIP,
        Cap::Hlt => KVM_CAP_HLT,
        Cap::MmuShadowCacheControl => KVM_CAP_MMU_SHADOW_CACHE_CONTROL,
        Cap::UserMemory => KVM_CAP_USER_MEMORY,
        Cap::SetTssAddr => KVM_CAP_SET_TSS_ADDR,
        Cap::Vapic => KVM_CAP_VAPIC,
        Cap::ExtCpuid => KVM_CAP_EXT_CPUID,
        Cap::Clocksource => KVM_CAP_CLOCKSOURCE,
        Cap::NrVcpus => KVM_CAP_NR_VCPUS,
        Cap::NrMemslots => KVM_CAP_NR_MEMSLOTS,
        Cap::Pit => KVM_CAP_PIT,
        Cap::NopIoDelay => KVM_CAP_NOP_IO_DELAY,
        Cap::PvMmu => KVM_CAP_PV_MMU,
        Cap::MpState => KVM_CAP_MP_STATE,
        Cap::CoalescedMmio => KVM_CAP_COALESCED_MMIO,
        Cap::SyncMmu => KVM_CAP_SYNC_MMU,
        Cap::Iommu => KVM_CAP_IOMMU,
        Cap::DestroyMemoryRegionWorks => KVM_CAP_DESTROY_MEMORY_REGION_WORKS,
        Cap::UserNmi => KVM_CAP_USER_NMI,
        Cap::SetGuestDebug => KVM_CAP_SET_GUEST_DEBUG,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::ReinjectControl => KVM_CAP_REINJECT_CONTROL,
        Cap::IrqRouting => KVM_CAP_IRQ_ROUTING,
        Cap::IrqInjectStatus => KVM_CAP_IRQ_INJECT_STATUS,
        Cap::AssignDevIrq => KVM_CAP_ASSIGN_DEV_IRQ,
        Cap::JoinMemoryRegionsWorks => KVM_CAP_JOIN_MEMORY_REGIONS_WORKS,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::Mce => KVM_CAP_MCE,
        Cap::Irqfd => KVM_CAP_IRQFD,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::Pit2 => KVM_CAP_PIT2,
        Cap::SetBootCpuId => KVM_CAP_SET_BOOT_CPU_ID,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::PitState2 => KVM_CAP_PIT_STATE2,
        Cap::Ioeventfd => KVM_CAP_IOEVENTFD,
        Cap::SetIdentityMapAddr => KVM_CAP_SET_IDENTITY_MAP_ADDR,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::XenHvm => KVM_CAP_XEN_HVM,
        Cap::AdjustClock => KVM_CAP_ADJUST_CLOCK,
        Cap::InternalErrorData => KVM_CAP_INTERNAL_ERROR_DATA,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::VcpuEvents => KVM_CAP_VCPU_EVENTS,
        Cap::S390Psw => KVM_CAP_S390_PSW,
        Cap::PpcSegstate => KVM_CAP_PPC_SEGSTATE,
        Cap::Hyperv => KVM_CAP_HYPERV,
        Cap::HypervVapic => KVM_CAP_HYPERV_VAPIC,
        Cap::HypervSpin => KVM_CAP_HYPERV_SPIN,
        Cap::PciSegment => KVM_CAP_PCI_SEGMENT,
        Cap::PpcPairedSingles => KVM_CAP_PPC_PAIRED_SINGLES,
        Cap::IntrShadow => KVM_CAP_INTR_SHADOW,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::Debugregs => KVM_CAP_DEBUGREGS,
        Cap::X86RobustSinglestep => KVM_CAP_X86_ROBUST_SINGLESTEP,
        Cap::PpcOsi => KVM_CAP_PPC_OSI,
        Cap::PpcUnsetIrq => KVM_CAP_PPC_UNSET_IRQ,
        Cap::EnableCap => KVM_CAP_ENABLE_CAP,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::Xsave => KVM_CAP_XSAVE,
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        Cap::Xcrs => KVM_CAP_XCRS,
        Cap::PpcGetPvinfo => KVM_CAP_PPC_GET_PVINFO,
        Cap::PpcIrqLevel => KVM_CAP_PPC_IRQ_LEVEL,
        Cap::AsyncPf => KVM_CAP_ASYNC_PF,
        Cap::TscControl => KVM_CAP_TSC_CONTROL,
        Cap::GetTscKhz => KVM_CAP_GET_TSC_KHZ,
        Cap::PpcBookeSregs => KVM_CAP_PPC_BOOKE_SREGS,
        Cap::SpaprTce => KVM_CAP_SPAPR_TCE,
        Cap::PpcSmt => KVM_CAP_PPC_SMT,
        Cap::PpcRma => KVM_CAP_PPC_RMA,
        Cap::MaxVcpus => KVM_CAP_MAX_VCPUS,
        Cap::PpcHior => KVM_CAP_PPC_HIOR,
        Cap::PpcPapr => KVM_CAP_PPC_PAPR,
        Cap::SwTlb => KVM_CAP_SW_TLB,
        Cap::OneReg => KVM_CAP_ONE_REG,
        Cap::S390Gmap => KVM_CAP_S390_GMAP,
        Cap::TscDeadlineTimer => KVM_CAP_TSC_DEADLINE_TIMER,
        Cap::S390Ucontrol => KVM_CAP_S390_UCONTROL,
        Cap::SyncRegs => KVM_CAP_SYNC_REGS,
        Cap::Pci23 => KVM_CAP_PCI_2_3,
        Cap::KvmclockCtrl => KVM_CAP_KVMCLOCK_CTRL,
        Cap::SignalMsi => KVM_CAP_SIGNAL_MSI,
        Cap::PpcGetSmmuInfo => KVM_CAP_PPC_GET_SMMU_INFO,
        Cap::S390Cow => KVM_CAP_S390_COW,
        Cap::PpcAllocHtab => KVM_CAP_PPC_ALLOC_HTAB,
        Cap::ReadonlyMem => KVM_CAP_READONLY_MEM,
        Cap::IrqfdResample => KVM_CAP_IRQFD_RESAMPLE,
        Cap::PpcBookeWatchdog => KVM_CAP_PPC_BOOKE_WATCHDOG,
        Cap::PpcHtabFd => KVM_CAP_PPC_HTAB_FD,
        Cap::S390CssSupport => KVM_CAP_S390_CSS_SUPPORT,
        Cap::PpcEpr => KVM_CAP_PPC_EPR,
        Cap::ArmPsci => KVM_CAP_ARM_PSCI,
        Cap::ArmSetDeviceAddr => KVM_CAP_ARM_SET_DEVICE_ADDR,
        Cap::DeviceCtrl => KVM_CAP_DEVICE_CTRL,
        Cap::IrqMpic => KVM_CAP_IRQ_MPIC,
        Cap::PpcRtas => KVM_CAP_PPC_RTAS,
        Cap::IrqXics => KVM_CAP_IRQ_XICS,
        Cap::ArmEl132bit => KVM_CAP_ARM_EL1_32BIT,
        Cap::SpaprMultitce => KVM_CAP_SPAPR_MULTITCE,
        Cap::ExtEmulCpuid => KVM_CAP_EXT_EMUL_CPUID,
        Cap::HypervTime => KVM_CAP_HYPERV_TIME,
        Cap::IoapicPolarityIgnored => KVM_CAP_IOAPIC_POLARITY_IGNORED,
        Cap::EnableCapVm => KVM_CAP_ENABLE_CAP_VM,
        Cap::S390Irqchip => KVM_CAP_S390_IRQCHIP,
        Cap::IoeventfdNoLength => KVM_CAP_IOEVENTFD_NO_LENGTH,
        Cap::VmAttributes => KVM_CAP_VM_ATTRIBUTES,
        Cap::ArmPsci02 => KVM_CAP_ARM_PSCI_0_2,
        Cap::PpcFixupHcall => KVM_CAP_PPC_FIXUP_HCALL,
        Cap::PpcEnableHcall => KVM_CAP_PPC_ENABLE_HCALL,
        Cap::CheckExtensionVm => KVM_CAP_CHECK_EXTENSION_VM,
        Cap::S390UserSigp => KVM_CAP_S390_USER_SIGP,
        Cap::ImmediateExit => KVM_CAP_IMMEDIATE_EXIT,
    }
}
