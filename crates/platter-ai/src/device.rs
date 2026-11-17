//! Where the model is loaded: the choice between a graphics card and the processor. Pure, so it is
//! tested without a graphics card.

use crate::engine::GpuDevice;
use crate::spec::ModelSpec;

/// The card to load `spec` on, or `None` for the processor: with `use_gpu` on, the discrete card with the
/// most free video memory, when that is at least the model's [`ModelSpec::gpu_video_bytes`]. An
/// integrated card is never chosen: it shares the system's memory (its "free" memory is the system's) and
/// is rarely faster than the processor.
pub fn choose<'a>(gpus: &'a [GpuDevice], use_gpu: bool, spec: &ModelSpec) -> Option<&'a GpuDevice> {
    if !use_gpu {
        return None;
    }

    gpus.iter()
        .filter(|card| !card.integrated && card.free_bytes >= spec.gpu_video_bytes)
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

        assert_eq!(choose(&gpus, true, &MODEL).map(|card| card.index), Some(1));
    }

    #[test]
    fn of_two_discrete_cards_the_one_with_more_free_memory_is_chosen() {
        let gpus = [
            card(0, 6_000, false),
            card(1, 20_000, false),
            card(2, 20_000, false),
        ];

        assert_eq!(choose(&gpus, true, &MODEL).map(|card| card.index), Some(1));
    }

    #[test]
    fn the_processor_is_used_without_a_card_with_enough_memory() {
        let need_mib = MODEL.gpu_video_bytes >> 20;

        assert_eq!(choose(&[], true, &MODEL), None);
        assert_eq!(choose(&[card(0, need_mib - 1, false)], true, &MODEL), None);
        assert_eq!(choose(&[card(0, 64_000, true)], true, &MODEL), None);
        assert_eq!(
            choose(&[card(0, need_mib, false)], true, &MODEL).map(|card| card.index),
            Some(0)
        );
    }

    #[test]
    fn with_use_gpu_off_no_card_is_chosen() {
        assert_eq!(choose(&[card(0, 24_000, false)], false, &MODEL), None);
    }
}
