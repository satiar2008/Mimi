//! Small PCM jitter buffer and incremental resampling; no device, file or network access.
use std::collections::VecDeque;
pub struct SpeechPcm {
    samples: VecDeque<f32>,
    fraction: f64,
    primed: bool,
    finished: bool,
    target: usize,
    pub underruns: u64,
}
impl Default for SpeechPcm {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(8192),
            fraction: 0.0,
            primed: false,
            finished: false,
            target: 3600,
            underruns: 0,
        }
    }
}
impl SpeechPcm {
    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn should_fill(&self) -> bool {
        self.samples.len() < self.target.max(4096)
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
    pub fn finish(&mut self) {
        self.finished = true;
    }
    pub fn drained(&self) -> bool {
        self.finished && self.samples.is_empty()
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if !bytes.len().is_multiple_of(2) || bytes.len() / 2 + self.samples.len() > 8192 {
            return Err("output_audio_limit");
        }
        self.samples.extend(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0),
        );
        Ok(())
    }
    pub fn sample(&mut self, rate: u32) -> Option<f32> {
        if rate == 0 {
            return None;
        }
        if !self.primed && (self.samples.len() >= self.target || self.finished) {
            self.primed = true;
        }
        if !self.primed {
            return None;
        }
        if self.samples.is_empty() || (!self.finished && self.samples.len() < 2) {
            if !self.finished {
                self.underruns += 1;
                self.target = 6000;
            }
            self.primed = false;
            return None;
        }
        let a = self.samples[0];
        let b = self.samples.get(1).copied().unwrap_or(a);
        let value = a + (b - a) * self.fraction as f32;
        self.fraction += 24000.0 / rate as f64;
        while self.fraction >= 1.0 {
            self.fraction -= 1.0;
            self.samples.pop_front();
        }
        Some(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn underflow_does_not_skip_samples_and_final_short_tail_is_drained() {
        let mut pcm = SpeechPcm::default();
        pcm.push(&[1, 0, 2, 0]).unwrap();
        assert_eq!(pcm.sample(24000), None);
        pcm.finish();
        assert_eq!(pcm.sample(24000), Some(1.0 / 32768.0));
        assert_eq!(pcm.sample(24000), Some(2.0 / 32768.0));
        assert!(pcm.drained());
    }
    #[test]
    fn resampling_is_incremental_and_buffers_remain_bounded() {
        let mut pcm = SpeechPcm::default();
        pcm.push(&vec![0; 7200]).unwrap();
        let mut count = 0;
        while pcm.sample(48000).is_some() {
            count += 1;
        }
        assert_eq!(count, 7198);
        assert!(!pcm.drained());
        pcm.push(&[1, 0, 2, 0]).unwrap();
        assert_eq!(pcm.sample(48000), None);
        pcm.finish();
        while pcm.sample(48000).is_some() {
            count += 1;
        }
        assert_eq!(count, 7204);
        assert!(pcm.drained());
        assert!(SpeechPcm::default().push(&vec![0; 16386]).is_err());
    }
}
