use crate::audio::{
    AudioState, PacketClock, SAMPLE_WIDTH_16_ALT, SAMPLE_WIDTH_24_ALT, StreamDirection,
    bytes_per_audio_frame, closest_supported_rate,
};

#[embedded_test::tests]
mod tests {
    use super::*;

    #[test]
    fn sample_width_alt_settings_map_to_audio_frame_sizes() {
        assert_eq!(bytes_per_audio_frame(0), 0);
        assert_eq!(bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT), 4);
        assert_eq!(bytes_per_audio_frame(SAMPLE_WIDTH_24_ALT), 6);
        assert_eq!(bytes_per_audio_frame(3), 0);
    }

    #[test]
    fn unsupported_sample_rates_round_to_nearest_supported_rate() {
        assert_eq!(closest_supported_rate(44_100), 44_100);
        assert_eq!(closest_supported_rate(48_000), 48_000);
        assert_eq!(closest_supported_rate(88_200), 88_200);
        assert_eq!(closest_supported_rate(96_000), 96_000);
        assert_eq!(closest_supported_rate(45_000), 44_100);
        assert_eq!(closest_supported_rate(50_000), 48_000);
        assert_eq!(closest_supported_rate(90_000), 88_200);
        assert_eq!(closest_supported_rate(95_000), 96_000);
    }

    #[test]
    fn packet_clock_emits_fractional_44k1_cadence() {
        let mut clock = PacketClock::new();
        let mut total = 0;

        for index in 0..10 {
            let len = clock.next_len(44_100, bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT));
            total += len;
            if index == 9 {
                assert_eq!(len, 45 * 4);
            } else {
                assert_eq!(len, 44 * 4);
            }
        }

        assert_eq!(total, 441 * 4);
    }

    #[test]
    fn packet_clock_emits_fractional_88k2_cadence() {
        let mut clock = PacketClock::new();
        let mut total = 0;

        for index in 0..5 {
            let len = clock.next_len(88_200, bytes_per_audio_frame(SAMPLE_WIDTH_24_ALT));
            total += len;
            if index == 4 {
                assert_eq!(len, 89 * 6);
            } else {
                assert_eq!(len, 88 * 6);
            }
        }

        assert_eq!(total, 441 * 6);
    }

    #[test]
    fn packet_clock_resets_fractional_state_when_format_changes() {
        let mut clock = PacketClock::new();

        assert_eq!(
            clock.next_len(44_100, bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT)),
            44 * 4
        );
        assert_eq!(
            clock.next_len(48_000, bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT)),
            48 * 4
        );
        assert_eq!(
            clock.next_len(48_000, bytes_per_audio_frame(SAMPLE_WIDTH_24_ALT)),
            48 * 6
        );
    }

    #[test]
    fn loopback_only_matches_identical_active_formats() {
        let state = AudioState::new();
        assert!(!state.loopback_format_matches());

        state.set_alt(StreamDirection::Out, SAMPLE_WIDTH_16_ALT);
        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_16_ALT);
        assert!(state.loopback_format_matches());

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_24_ALT);
        assert!(!state.loopback_format_matches());

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_16_ALT);
        state.set_rate_hz(StreamDirection::In, 44_100);
        assert!(!state.loopback_format_matches());
    }
}
