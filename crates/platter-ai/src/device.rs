//! Where the model is loaded: the choice between a graphics card and the processor. Pure, so it is
//! tested without a graphics card.

use crate::engine::GpuDevice;
use crate::spec::ModelSpec;

/// Which graphics parts may be chosen. It depends on the machine's memory design, so the OS decides it
/// ([`GpuPolicy::THIS_OS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPolicy {
    /// Discrete cards only (Linux, Windows). An integrated part shares the system's memory (its "free"
    /// memory is the system's) and is rarely faster than the processor.
    DiscreteOnly,
    /// Every part counts (macOS). An Apple GPU has unified memory by design: it is the only graphics part
    /// there and is much faster than the processor, and the memory it reports is what Metal may use
    /// (`recommendedMaxWorkingSetSize` less what is already allocated).
    UnifiedMemory,
}

impl GpuPolicy {
    /// The policy of the OS this is built for.
    pub const THIS_OS: Self = if cfg!(target_os = "macos") {
        Self::UnifiedMemory
    } else {
        Self::DiscreteOnly
    };

    /// Whether the model's weights and the system's memory are one pool, so that a load on the part still
    /// needs the memory a load on the processor needs.
    pub const fn shares_system_memory(self) -> bool {
        matches!(self, Self::UnifiedMemory)
    }
}

/// The card to load `spec` on, or `None` for the processor: with `use_gpu` on, the card with the most free
/// video memory that `policy` allows, when that is at least the model's [`ModelSpec::gpu_video_bytes`].
pub fn choose<'a>(
    gpus: &'a [GpuDevice],
    use_gpu: bool,
    spec: &ModelSpec,
    policy: GpuPolicy,
) -> Option<&'a GpuDevice> {
    if !use_gpu {
        return None;
    }

    gpus.iter()
        .filter(|card| policy == GpuPolicy::UnifiedMemory || !card.integrated)
        .filter(|card| card.free_bytes >= spec.gpu_video_bytes)
        .max_by_key(|card| (card.free_bytes, std::cmp::Reverse(card.index)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::MODEL;

    fn card(index: usize, free_mib: u64, integrated: bool) -> GpuDevice {
        GpuDevice {
            index,
            name: format!("Vulkan{index}"),
            description: format!("card {index}"),
            total_bytes: (free_mib * 2) << 20,
            free_bytes: free_mib << 20,
            integrated,
        }
    }

    #[test]
    fn the_discrete_card_is_chosen_over_an_integrated_one_with_more_memory() {
        let gpus = [card(0, 32_000, true), card(1, 20_000, false)];

        assert_eq!(
            choose(&gpus, true, &MODEL, GpuPolicy::DiscreteOnly).map(|card| card.index),
            Some(1)
        );
    }

    #[test]
    fn of_two_discrete_cards_the_one_with_more_free_memory_is_chosen() {
        let gpus = [
            card(0, 6_000, false),
            card(1, 20_000, false),
            card(2, 20_000, false),
        ];

        assert_eq!(
            choose(&gpus, true, &MODEL, GpuPolicy::DiscreteOnly).map(|card| card.index),
            Some(1)
        );
    }

    #[test]
    fn the_processor_is_used_without_a_card_with_enough_memory() {
        let need_mib = MODEL.gpu_video_bytes >> 20;

        assert_eq!(choose(&[], true, &MODEL, GpuPolicy::DiscreteOnly), None);
        assert_eq!(
            choose(
                &[card(0, need_mib - 1, false)],
                true,
                &MODEL,
                GpuPolicy::DiscreteOnly
            ),
            None
        );
        assert_eq!(
            choose(&[card(0, 64_000, true)], true, &MODEL, GpuPolicy::DiscreteOnly),
            None
        );
        assert_eq!(
            choose(&[card(0, need_mib, false)], true, &MODEL, GpuPolicy::DiscreteOnly).map(|card| card.index),
            Some(0)
        );
    }

    #[test]
    fn with_use_gpu_off_no_card_is_chosen() {
        assert_eq!(
            choose(&[card(0, 24_000, false)], false, &MODEL, GpuPolicy::DiscreteOnly),
            None
        );
    }

    #[test]
    fn on_unified_memory_the_apple_gpu_is_chosen_with_or_without_the_integrated_flag() {
        let need_mib = MODEL.gpu_video_bytes >> 20;

        for integrated in [true, false] {
            let gpus = [card(0, need_mib, integrated)];

            assert_eq!(
                choose(&gpus, true, &MODEL, GpuPolicy::UnifiedMemory).map(|card| card.index),
                Some(0),
                "integrated: {integrated}"
            );
        }
    }

    #[test]
    fn on_unified_memory_the_part_still_needs_its_free_memory_and_the_switch_on() {
        let need_mib = MODEL.gpu_video_bytes >> 20;

        assert_eq!(
            choose(
                &[card(0, need_mib - 1, true)],
                true,
                &MODEL,
                GpuPolicy::UnifiedMemory
            ),
            None
        );
        assert_eq!(
            choose(&[card(0, 64_000, true)], false, &MODEL, GpuPolicy::UnifiedMemory),
            None
        );
        assert_eq!(choose(&[], true, &MODEL, GpuPolicy::UnifiedMemory), None);
    }

    #[test]
    fn the_same_integrated_part_is_refused_by_one_policy_and_taken_by_the_other() {
        let gpus = [card(0, 64_000, true)];

        assert_eq!(choose(&gpus, true, &MODEL, GpuPolicy::DiscreteOnly), None);
        assert!(choose(&gpus, true, &MODEL, GpuPolicy::UnifiedMemory).is_some());
    }

    #[test]
    fn only_the_unified_policy_shares_the_systems_memory() {
        assert!(GpuPolicy::UnifiedMemory.shares_system_memory());
        assert!(!GpuPolicy::DiscreteOnly.shares_system_memory());
    }
}
