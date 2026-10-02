use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::domain::{Allocation, CoreMask, PluginSummary};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("plugin '{0}' is not registered")]
    PluginNotFound(String),
    #[error("core mask is not allowed by plugin '{0}'")]
    MaskNotAllowed(String),
    #[error("requested NPU cores are busy")]
    CoresBusy,
    #[error("lease '{0}' was not found")]
    LeaseNotFound(String),
}

#[derive(Debug, Default)]
pub struct Scheduler {
    allocations: Vec<Allocation>,
    // Lease IDs are process-local: restart clears allocations and state.json
    // deliberately does not persist lease IDs, so restarting at 1 is safe.
    next_lease_id: u64,
}

impl Scheduler {
    pub fn allocations(&self) -> &[Allocation] {
        &self.allocations
    }

    pub fn allocate(
        &mut self,
        plugin: &PluginSummary,
        requested: CoreMask,
    ) -> Result<Allocation, ScheduleError> {
        let mask = if requested == CoreMask::Auto {
            self.select_mask(plugin)?
        } else {
            requested
        };

        if !plugin.allowed_masks.contains(&mask) {
            return Err(ScheduleError::MaskNotAllowed(plugin.id.clone()));
        }
        if self.occupied_bits() & mask.bits() != 0 {
            return Err(ScheduleError::CoresBusy);
        }

        self.next_lease_id += 1;
        let allocation = Allocation {
            lease_id: format!("lease-{:08}", self.next_lease_id),
            plugin_id: plugin.id.clone(),
            core_mask: mask,
            created_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        };
        self.allocations.push(allocation.clone());
        Ok(allocation)
    }

    pub fn release(&mut self, lease_id: &str) -> Result<Allocation, ScheduleError> {
        let index = self
            .allocations
            .iter()
            .position(|allocation| allocation.lease_id == lease_id)
            .ok_or_else(|| ScheduleError::LeaseNotFound(lease_id.to_owned()))?;
        Ok(self.allocations.remove(index))
    }

    pub fn occupied_bits(&self) -> u8 {
        self.allocations
            .iter()
            .fold(0, |bits, allocation| bits | allocation.core_mask.bits())
    }

    fn select_mask(&self, plugin: &PluginSummary) -> Result<CoreMask, ScheduleError> {
        let occupied = self.occupied_bits();
        // Prefer the manifest default, then the smallest available single core,
        // and use the dual-core mask only when explicitly allowed and free.
        let preferred = std::iter::once(plugin.default_mask)
            .chain([CoreMask::Core0, CoreMask::Core1, CoreMask::Core0_1]);

        preferred
            .filter(|mask| *mask != CoreMask::Auto)
            .find(|mask| plugin.allowed_masks.contains(mask) && occupied & mask.bits() == 0)
            .ok_or(ScheduleError::CoresBusy)
    }
}
