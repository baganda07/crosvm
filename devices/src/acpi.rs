// Copyright 2019 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::{BusAccessInfo, BusDevice, BusResumeDevice};
use acpi_tables::{aml, aml::Aml};
use base::{error, warn, Event};
use std::sync::Arc;
use sync::Mutex;
use vm_control::PmResource;

struct Pm1Resource {
    status: u16,
    enable: u16,
    control: u16,
}

struct GpeResource {
    status: [u8; ACPIPM_RESOURCE_GPE0_BLK_LEN as usize / 2],
    enable: [u8; ACPIPM_RESOURCE_GPE0_BLK_LEN as usize / 2],
}

/// ACPI PM resource for handling OS suspend/resume request
#[allow(dead_code)]
pub struct ACPIPMResource {
    sci_evt: Event,
    sci_evt_resample: Event,
    suspend_evt: Event,
    exit_evt: Event,
    pm1: Arc<Mutex<Pm1Resource>>,
    gpe0: Arc<Mutex<GpeResource>>,
}

impl ACPIPMResource {
    /// Constructs ACPI Power Management Resouce.
    #[allow(dead_code)]
    pub fn new(
        sci_evt: Event,
        sci_evt_resample: Event,
        suspend_evt: Event,
        exit_evt: Event,
    ) -> ACPIPMResource {
        let pm1 = Pm1Resource {
            status: 0,
            enable: 0,
            control: 0,
        };
        let gpe0 = GpeResource {
            status: Default::default(),
            enable: Default::default(),
        };

        ACPIPMResource {
            sci_evt,
            sci_evt_resample,
            suspend_evt,
            exit_evt,
            pm1: Arc::new(Mutex::new(pm1)),
            gpe0: Arc::new(Mutex::new(gpe0)),
        }
    }
}

impl Pm1Resource {
    fn trigger_sci(&self, sci_evt: &Event) {
        if self.status
            & self.enable
            & (BITMASK_PM1EN_GBL_EN
                | BITMASK_PM1EN_PWRBTN_EN
                | BITMASK_PM1EN_SLPBTN_EN
                | BITMASK_PM1EN_RTC_EN)
            != 0
        {
            if let Err(e) = sci_evt.write(1) {
                error!("ACPIPM: failed to trigger sci event for pm1: {}", e);
            }
        }
    }
}

impl GpeResource {
    fn trigger_sci(&self, sci_evt: &Event) {
        if (0..self.status.len()).any(|i| self.status[i] & self.enable[i] != 0) {
            if let Err(e) = sci_evt.write(1) {
                error!("ACPIPM: failed to trigger sci event for gpe: {}", e);
            }
        }
    }
}

/// the ACPI PM register length.
pub const ACPIPM_RESOURCE_EVENTBLK_LEN: u8 = 4;
pub const ACPIPM_RESOURCE_CONTROLBLK_LEN: u8 = 2;
pub const ACPIPM_RESOURCE_GPE0_BLK_LEN: u8 = 64;
pub const ACPIPM_RESOURCE_LEN: u8 = ACPIPM_RESOURCE_EVENTBLK_LEN + 4 + ACPIPM_RESOURCE_GPE0_BLK_LEN;

/// ACPI PM register value definitions

/// 4.8.4.1.1 PM1 Status Registers, ACPI Spec Version 6.4
/// Register Location: <PM1a_EVT_BLK / PM1b_EVT_BLK> System I/O or Memory Space (defined in FADT)
/// Size: PM1_EVT_LEN / 2 (defined in FADT)
const PM1_STATUS: u16 = 0;

/// 4.8.4.1.2 PM1Enable Registers, ACPI Spec Version 6.4
/// Register Location: <<PM1a_EVT_BLK / PM1b_EVT_BLK> + PM1_EVT_LEN / 2 System I/O or Memory Space
/// (defined in FADT)
/// Size: PM1_EVT_LEN / 2 (defined in FADT)
const PM1_ENABLE: u16 = PM1_STATUS + (ACPIPM_RESOURCE_EVENTBLK_LEN as u16 / 2);

/// 4.8.4.2.1 PM1 Control Registers, ACPI Spec Version 6.4
/// Register Location: <PM1a_CNT_BLK / PM1b_CNT_BLK> System I/O or Memory Space (defined in FADT)
/// Size: PM1_CNT_LEN (defined in FADT)
const PM1_CONTROL: u16 = PM1_STATUS + ACPIPM_RESOURCE_EVENTBLK_LEN as u16;

/// 4.8.5.1 General-Purpose Event Register Blocks, ACPI Spec Version 6.4
/// - Each register block contains two registers: an enable and a status register.
/// - Each register block is 32-bit aligned.
/// - Each register in the block is accessed as a byte.

/// 4.8.5.1.1 General-Purpose Event 0 Register Block, ACPI Spec Version 6.4
/// This register block consists of two registers: The GPE0_STS and the GPE0_EN registers. Each
/// register’s length is defined to be half the length of the GPE0 register block, and is described
/// in the ACPI FADT’s GPE0_BLK and GPE0_BLK_LEN operators.

/// 4.8.5.1.1.1 General-Purpose Event 0 Status Register, ACPI Spec Version 6.4
/// Register Location: <GPE0_STS> System I/O or System Memory Space (defined in FADT)
/// Size: GPE0_BLK_LEN/2 (defined in FADT)
const GPE0_STATUS: u16 = PM1_STATUS + ACPIPM_RESOURCE_EVENTBLK_LEN as u16 + 4; // ensure alignment

/// 4.8.5.1.1.2 General-Purpose Event 0 Enable Register, ACPI Spec Version 6.4
/// Register Location: <GPE0_EN> System I/O or System Memory Space (defined in FADT)
/// Size: GPE0_BLK_LEN/2 (defined in FADT)
const GPE0_ENABLE: u16 = GPE0_STATUS + (ACPIPM_RESOURCE_GPE0_BLK_LEN as u16 / 2);

const BITMASK_PM1STS_PWRBTN_STS: u16 = 1 << 8;
const BITMASK_PM1EN_GBL_EN: u16 = 1 << 5;
const BITMASK_PM1EN_PWRBTN_EN: u16 = 1 << 8;
const BITMASK_PM1EN_SLPBTN_EN: u16 = 1 << 9;
const BITMASK_PM1EN_RTC_EN: u16 = 1 << 10;
const BITMASK_PM1CNT_SLEEP_ENABLE: u16 = 0x2000;
const BITMASK_PM1CNT_WAKE_STATUS: u16 = 0x8000;

#[cfg(not(feature = "direct"))]
const BITMASK_PM1CNT_SLEEP_TYPE: u16 = 0x1C00;
#[cfg(not(feature = "direct"))]
const SLEEP_TYPE_S1: u16 = 1 << 10;
#[cfg(not(feature = "direct"))]
const SLEEP_TYPE_S5: u16 = 0 << 10;

impl PmResource for ACPIPMResource {
    fn pwrbtn_evt(&mut self) {
        let mut pm1 = self.pm1.lock();

        pm1.status |= BITMASK_PM1STS_PWRBTN_STS;
        pm1.trigger_sci(&self.sci_evt);
    }

    fn gpe_evt(&mut self, gpe: u32) {
        let mut gpe0 = self.gpe0.lock();

        let byte = gpe as usize / 8;
        if byte >= gpe0.status.len() {
            error!("gpe_evt: GPE register {} does not exist", byte);
            return;
        }
        gpe0.status[byte] |= 1 << (gpe % 8);
        gpe0.trigger_sci(&self.sci_evt);
    }
}

const PM1_STATUS_LAST: u16 = PM1_STATUS + (ACPIPM_RESOURCE_EVENTBLK_LEN as u16 / 2) - 1;
const PM1_ENABLE_LAST: u16 = PM1_ENABLE + (ACPIPM_RESOURCE_EVENTBLK_LEN as u16 / 2) - 1;
const PM1_CONTROL_LAST: u16 = PM1_CONTROL + ACPIPM_RESOURCE_CONTROLBLK_LEN as u16 - 1;
const GPE0_STATUS_LAST: u16 = GPE0_STATUS + (ACPIPM_RESOURCE_GPE0_BLK_LEN as u16 / 2) - 1;
const GPE0_ENABLE_LAST: u16 = GPE0_ENABLE + (ACPIPM_RESOURCE_GPE0_BLK_LEN as u16 / 2) - 1;

impl BusDevice for ACPIPMResource {
    fn debug_label(&self) -> String {
        "ACPIPMResource".to_owned()
    }

    fn read(&mut self, info: BusAccessInfo, data: &mut [u8]) {
        match info.offset as u16 {
            // Accesses to the PM1 registers are done through byte or word accesses
            PM1_STATUS..=PM1_STATUS_LAST => {
                if data.len() > std::mem::size_of::<u16>()
                    || info.offset + data.len() as u64 > (PM1_STATUS_LAST + 1).into()
                {
                    warn!("ACPIPM: bad read size: {}", data.len());
                    return;
                }
                let offset = (info.offset - PM1_STATUS as u64) as usize;
                data.copy_from_slice(
                    &self.pm1.lock().status.to_ne_bytes()[offset..offset + data.len()],
                );
            }
            PM1_ENABLE..=PM1_ENABLE_LAST => {
                if data.len() > std::mem::size_of::<u16>()
                    || info.offset + data.len() as u64 > (PM1_ENABLE_LAST + 1).into()
                {
                    warn!("ACPIPM: bad read size: {}", data.len());
                    return;
                }
                let offset = (info.offset - PM1_ENABLE as u64) as usize;
                data.copy_from_slice(
                    &self.pm1.lock().enable.to_ne_bytes()[offset..offset + data.len()],
                );
            }
            PM1_CONTROL..=PM1_CONTROL_LAST => {
                if data.len() > std::mem::size_of::<u16>()
                    || info.offset + data.len() as u64 > (PM1_CONTROL_LAST + 1).into()
                {
                    warn!("ACPIPM: bad read size: {}", data.len());
                    return;
                }
                let offset = (info.offset - PM1_CONTROL as u64) as usize;
                data.copy_from_slice(
                    &self.pm1.lock().control.to_ne_bytes()[offset..offset + data.len()],
                );
            }
            // OSPM accesses GPE registers through byte accesses (regardless of their length)
            GPE0_STATUS..=GPE0_STATUS_LAST => {
                if data.len() > std::mem::size_of::<u8>()
                    || info.offset + data.len() as u64 > (GPE0_STATUS_LAST + 1).into()
                {
                    warn!("ACPIPM: bad read size: {}", data.len());
                    return;
                }
                data[0] = self.gpe0.lock().status[(info.offset - GPE0_STATUS as u64) as usize];
            }
            GPE0_ENABLE..=GPE0_ENABLE_LAST => {
                if data.len() > std::mem::size_of::<u8>()
                    || info.offset + data.len() as u64 > (GPE0_ENABLE_LAST + 1).into()
                {
                    warn!("ACPIPM: bad read size: {}", data.len());
                    return;
                }
                data[0] = self.gpe0.lock().enable[(info.offset - GPE0_ENABLE as u64) as usize];
            }
            _ => {
                warn!("ACPIPM: Bad read from {}", info);
            }
        }
    }

    fn write(&mut self, info: BusAccessInfo, data: &[u8]) {
        match info.offset as u16 {
            // Accesses to the PM1 registers are done through byte or word accesses
            PM1_STATUS..=PM1_STATUS_LAST => {
                if data.len() > std::mem::size_of::<u16>()
                    || info.offset + data.len() as u64 > (PM1_STATUS_LAST + 1).into()
                {
                    warn!("ACPIPM: bad write size: {}", data.len());
                    return;
                }
                let offset = (info.offset - PM1_STATUS as u64) as usize;

                let mut pm1 = self.pm1.lock();
                let mut v = pm1.status.to_ne_bytes();
                for (i, j) in (offset..offset + data.len()).enumerate() {
                    v[j] &= !data[i];
                }
                pm1.status = u16::from_ne_bytes(v);
            }
            PM1_ENABLE..=PM1_ENABLE_LAST => {
                if data.len() > std::mem::size_of::<u16>()
                    || info.offset + data.len() as u64 > (PM1_ENABLE_LAST + 1).into()
                {
                    warn!("ACPIPM: bad write size: {}", data.len());
                    return;
                }
                let offset = (info.offset - PM1_ENABLE as u64) as usize;

                let mut pm1 = self.pm1.lock();
                let mut v = pm1.enable.to_ne_bytes();
                for (i, j) in (offset..offset + data.len()).enumerate() {
                    v[j] = data[i];
                }
                pm1.enable = u16::from_ne_bytes(v);
                pm1.trigger_sci(&self.sci_evt);
            }
            PM1_CONTROL..=PM1_CONTROL_LAST => {
                if data.len() > std::mem::size_of::<u16>()
                    || info.offset + data.len() as u64 > (PM1_CONTROL_LAST + 1).into()
                {
                    warn!("ACPIPM: bad write size: {}", data.len());
                    return;
                }
                let offset = (info.offset - PM1_CONTROL as u64) as usize;

                let mut pm1 = self.pm1.lock();

                let mut v = pm1.control.to_ne_bytes();
                for (i, j) in (offset..offset + data.len()).enumerate() {
                    v[j] = data[i];
                }
                let val = u16::from_ne_bytes(v);

                // SLP_EN is a write-only bit and reads to it always return a zero
                if (val & BITMASK_PM1CNT_SLEEP_ENABLE) != 0 {
                    // only support S5 in direct mode
                    #[cfg(feature = "direct")]
                    if let Err(e) = self.exit_evt.write(1) {
                        error!("ACPIPM: failed to trigger exit event: {}", e);
                    }
                    #[cfg(not(feature = "direct"))]
                    match val & BITMASK_PM1CNT_SLEEP_TYPE {
                        SLEEP_TYPE_S1 => {
                            if let Err(e) = self.suspend_evt.write(1) {
                                error!("ACPIPM: failed to trigger suspend event: {}", e);
                            }
                        }
                        SLEEP_TYPE_S5 => {
                            if let Err(e) = self.exit_evt.write(1) {
                                error!("ACPIPM: failed to trigger exit event: {}", e);
                            }
                        }
                        _ => error!(
                            "ACPIPM: unknown SLP_TYP written: {}",
                            (val & BITMASK_PM1CNT_SLEEP_TYPE) >> 10
                        ),
                    }
                }
                pm1.control = val & !BITMASK_PM1CNT_SLEEP_ENABLE;
            }
            // OSPM accesses GPE registers through byte accesses (regardless of their length)
            GPE0_STATUS..=GPE0_STATUS_LAST => {
                if data.len() > std::mem::size_of::<u8>()
                    || info.offset + data.len() as u64 > (GPE0_STATUS_LAST + 1).into()
                {
                    warn!("ACPIPM: bad write size: {}", data.len());
                    return;
                }
                self.gpe0.lock().status[(info.offset - GPE0_STATUS as u64) as usize] &= !data[0];
            }
            GPE0_ENABLE..=GPE0_ENABLE_LAST => {
                if data.len() > std::mem::size_of::<u8>()
                    || info.offset + data.len() as u64 > (GPE0_ENABLE_LAST + 1).into()
                {
                    warn!("ACPIPM: bad write size: {}", data.len());
                    return;
                }

                let mut gpe = self.gpe0.lock();
                gpe.enable[(info.offset - GPE0_ENABLE as u64) as usize] = data[0];
                gpe.trigger_sci(&self.sci_evt);
            }
            _ => {
                warn!("ACPIPM: Bad write to {}", info);
            }
        };
    }
}

impl BusResumeDevice for ACPIPMResource {
    fn resume_imminent(&mut self) {
        self.pm1.lock().status |= BITMASK_PM1CNT_WAKE_STATUS;
    }
}

impl Aml for ACPIPMResource {
    fn to_aml_bytes(&self, bytes: &mut Vec<u8>) {
        // S1
        aml::Name::new(
            "_S1_".into(),
            &aml::Package::new(vec![&aml::ONE, &aml::ONE, &aml::ZERO, &aml::ZERO]),
        )
        .to_aml_bytes(bytes);

        // S5
        aml::Name::new(
            "_S5_".into(),
            &aml::Package::new(vec![&aml::ZERO, &aml::ZERO, &aml::ZERO, &aml::ZERO]),
        )
        .to_aml_bytes(bytes);
    }
}
