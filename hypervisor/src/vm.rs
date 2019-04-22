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

use std::{io, result};
use std::fs::File;
use std::os::unix::io::{AsRawFd, RawFd};

use crate::vcpu::Vcpu;

pub use crate::x86_64::{ PitConfig, IoEventAddress, CreateDevice, DeviceAttr };

// TODO: should move to arm specific file.
#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
pub use kvm_bindings::kvm_vcpu_init as VcpuInit;
#[cfg(target_arch = "aarch64")]
pub use kvm_bindings::KVM_ARM_VCPU_PSCI_0_2 as ARM_VCPU_PSCI_0_2;
#[cfg(target_arch = "aarch64")]
pub use kvm_bindings::KVM_ARM_VCPU_POWER_OFF as ARM_VCPU_POWER_OFF;

pub type Result<T> = result::Result<T, io::Error>;

pub struct DeviceFd {
    fd: File,
}

impl DeviceFd {
    pub fn new(f: File) -> Self {
        DeviceFd { fd: f }
    }
}

impl AsRawFd for DeviceFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

pub trait Vm {
    fn create_vcpu(&self, id: u8) -> Result<Box<Vcpu + Send>>;
    fn set_user_memory_region(&self,
                              slot: u32,
                              guest_phys_addr: u64,
                              memory_size: u64,
                              userspace_addr: u64,
                              flags: u32) -> Result<()>;
    fn set_tss_address(&self, offset: usize) -> Result<()>;
    fn create_irq_chip(&self) -> Result<()>;
    fn register_ioevent(&self,
                        fd: RawFd,
                        addr: &IoEventAddress,
                        datamatch: u64) -> Result<()>;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn create_pit2(&self, pit_config: PitConfig) -> Result<()>;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn get_dirty_log(&self, slot: u32, memory_size: usize) -> Result<Vec<u64>>;
    fn register_irqfd(&self, fd: RawFd, gsi: u32) -> Result<()>;
    fn create_device(&self, device: &mut CreateDevice) -> Result<DeviceFd>;
    #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
    fn get_preferred_target(&self, vi: &mut VcpuInit) -> Result<()>;
}
