//! Resample captured mono PCM to the 16 kHz float32 Whisper expects, and
//! drop a stale worker `ready` after a restart.
//!
//! Port of `src/renderer/lib/stt/resample.ts`. Linear interpolation. The
//! ready-drop is the dictation controller rule: a `ready` whose model id
//! does not match the model the current worker was asked to load is ignored
//! (a restarted worker's late message must not clobber the new generation).

pub const WHISPER_SAMPLE_RATE: f64 = 16000.0;

/// Resample mono Float32 PCM to 16 kHz.
///
/// Returns the same allocation when `input_rate` is already 16 kHz or the
/// buffer is empty. Values are passed through; they are assumed to be in
/// [-1, 1].
pub fn resample_to_16k_mono(input: Vec<f32>, input_rate: f64) -> Result<Vec<f32>, String> {
    if !input_rate.is_finite() || input_rate <= 0.0 {
        return Err(format!(
            "resampleTo16kMono: invalid inputRate {}",
            format_rate(input_rate)
        ));
    }
    if input_rate == WHISPER_SAMPLE_RATE || input.is_empty() {
        return Ok(input);
    }
    let ratio = input_rate / WHISPER_SAMPLE_RATE;
    let out_length = (input.len() as f64 / ratio).round().max(1.0) as usize;
    let mut output = vec![0.0f32; out_length];
    let last = input.len() - 1;
    for i in 0..out_length {
        let src_pos = i as f64 * ratio;
        let i0 = src_pos.floor() as usize;
        let i1 = (i0 + 1).min(last);
        let frac = (src_pos - i0 as f64) as f32;
        output[i] = input[i0] * (1.0 - frac) + input[i1] * frac;
    }
    Ok(output)
}

fn format_rate(rate: f64) -> String {
    if rate.is_nan() {
        "NaN".to_string()
    } else if rate == 0.0 {
        "0".to_string()
    } else if rate.fract() == 0.0 && rate.abs() < 1e15 {
        format!("{}", rate as i64)
    } else {
        // JS String(number) for typical rates.
        let s = format!("{rate}");
        s
    }
}

/// Concatenate Float32 frames into one contiguous buffer.
pub fn concat_float32(frames: &[Vec<f32>]) -> Vec<f32> {
    let total: usize = frames.iter().map(|f| f.len()).sum();
    let mut out = Vec::with_capacity(total);
    for f in frames {
        out.extend_from_slice(f);
    }
    out
}

/// Generation counter for the STT worker. A restart bumps `generation` and
/// records the model that generation was asked to load. A `ready` message
/// is applied only when both match — a ready from the previous worker is
/// stale and dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SttWorkerSlot {
    pub generation: u64,
    pub model_id: Option<String>,
}

impl SttWorkerSlot {
    pub fn new() -> Self {
        Self {
            generation: 0,
            model_id: None,
        }
    }

    /// Start (or restart) the worker for `model_id`. Returns the generation
    /// stamped onto messages this worker is allowed to emit.
    pub fn restart(&mut self, model_id: &str) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.model_id = Some(model_id.to_string());
        self.generation
    }

    /// `ready` applies only for the live generation and the model it loaded.
    /// A restarted worker's late ready (old generation, or a different model
    /// id) is dropped.
    pub fn accept_ready(&self, generation: u64, model_id: &str) -> bool {
        self.generation == generation && self.model_id.as_deref() == Some(model_id)
    }
}

impl Default for SttWorkerSlot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_same_buffer_when_already_16k() {
        let input = vec![0.0, 0.5, -0.5, 1.0];
        let ptr = input.as_ptr();
        let out = resample_to_16k_mono(input, WHISPER_SAMPLE_RATE).unwrap();
        assert_eq!(out.as_ptr(), ptr);
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = resample_to_16k_mono(Vec::new(), 48000.0).unwrap();
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn downsamples_48k_to_16k_at_a_third() {
        let input = vec![0.0; 4800];
        let out = resample_to_16k_mono(input, 48000.0).unwrap();
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn downsamples_44_1k_to_roughly_the_right_length() {
        let input = vec![0.0; 44100];
        let out = resample_to_16k_mono(input, 44100.0).unwrap();
        assert_eq!(out.len(), 16000);
    }

    #[test]
    fn preserves_a_constant_dc_signal_exactly() {
        let input = vec![0.42; 48000];
        let out = resample_to_16k_mono(input, 48000.0).unwrap();
        for v in out {
            assert!((v - 0.42).abs() < 1e-6);
        }
    }

    #[test]
    fn linearly_interpolates_between_samples() {
        let input = vec![0.0, 1.0];
        let out = resample_to_16k_mono(input, 8000.0).unwrap();
        assert!(out.len() >= 2);
        assert!(out[0].abs() < 1e-6);
        for v in out {
            assert!(v >= -1e-6 && v <= 1.0 + 1e-6);
        }
    }

    #[test]
    fn rejects_a_non_positive_input_rate() {
        let err0 = resample_to_16k_mono(vec![1.0], 0.0).unwrap_err();
        assert!(err0.contains("resampleTo16kMono: invalid inputRate 0"));
        let err1 = resample_to_16k_mono(vec![1.0], -1.0).unwrap_err();
        assert!(err1.contains("resampleTo16kMono: invalid inputRate -1"));
    }

    #[test]
    fn concat_float32_joins_frames_in_order() {
        let out = concat_float32(&[vec![1.0, 2.0], vec![3.0], vec![4.0, 5.0]]);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn concat_float32_of_nothing_is_empty() {
        assert_eq!(concat_float32(&[]).len(), 0);
    }

    #[test]
    fn worker_restart_drops_a_stale_ready_message() {
        let mut slot = SttWorkerSlot::new();
        let gen1 = slot.restart("tiny");
        assert!(slot.accept_ready(gen1, "tiny"));
        // Model switch / worker restart: the previous worker's ready is stale.
        let gen2 = slot.restart("base");
        assert!(!slot.accept_ready(gen1, "tiny"));
        assert!(!slot.accept_ready(gen1, "base"));
        assert!(!slot.accept_ready(gen2, "tiny"));
        assert!(slot.accept_ready(gen2, "base"));
    }
}
