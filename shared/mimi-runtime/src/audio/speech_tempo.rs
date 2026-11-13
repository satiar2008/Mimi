//! Bounded 24 kHz mono WSOLA tempo adjustment, outside the realtime device callback.
//! Analysis hops change duration; aligned overlapping grains preserve pitch.
use std::collections::VecDeque;
const HOP: usize = 960;
const OVERLAP: usize = 480;
const SEARCH: usize = 240;
pub struct SpeechTempo {
    speed: f64,
    input: VecDeque<f32>,
    base: usize,
    next: f64,
    tail: Vec<f32>,
    received: usize,
    emitted: usize,
}
impl SpeechTempo {
    pub fn new(speed: f64) -> Result<Self, &'static str> {
        if !(0.7..=1.6).contains(&speed) {
            return Err("output_invalid_settings");
        }
        Ok(Self {
            speed,
            input: VecDeque::with_capacity(8192),
            base: 0,
            next: 0.0,
            tail: Vec::new(),
            received: 0,
            emitted: 0,
        })
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, &'static str> {
        if !bytes.len().is_multiple_of(2) || bytes.len() > 4096 {
            return Err("output_audio_limit");
        }
        self.received += bytes.len() / 2;
        if self.speed == 1.0 {
            self.emitted += bytes.len() / 2;
            return Ok(vec![bytes.to_vec()]);
        }
        self.input.extend(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b) as f32),
        );
        if self.input.len() > 8192 {
            return Err("output_audio_limit");
        }
        Ok(self.pump(false))
    }
    pub fn finish(&mut self) -> Vec<Vec<u8>> {
        self.pump(true)
    }
    fn pump(&mut self, finished: bool) -> Vec<Vec<u8>> {
        let mut output = Vec::new();
        let limit = (self.received as f64 / self.speed).round() as usize;
        while self.emitted < limit {
            let center = self.next.round() as usize;
            if !finished && center + SEARCH + HOP + OVERLAP > self.base + self.input.len() {
                break;
            }
            let low = center.saturating_sub(SEARCH).max(self.base);
            let high =
                (center + SEARCH).min((self.base + self.input.len()).saturating_sub(HOP + OVERLAP));
            let mut selected = center.max(self.base).min(self.base + self.input.len());
            if !self.tail.is_empty() && low <= high {
                let mut score = f64::NEG_INFINITY;
                // Normalized correlation avoids preferring a louder grain.
                for candidate in (low..=high).step_by(4) {
                    let mut dot = 0.0f64;
                    let mut energy = 1.0f64;
                    for i in (0..OVERLAP).step_by(4) {
                        let value = self.input[candidate - self.base + i] as f64;
                        dot += self.tail[i] as f64 * value;
                        energy += value * value;
                    }
                    let value = dot / energy.sqrt();
                    if value > score {
                        score = value;
                        selected = candidate;
                    }
                }
            }
            let count = HOP.min(limit - self.emitted);
            let at = selected - self.base;
            let grain = |i: usize| self.input.get(at + i).copied().unwrap_or(0.0);
            let mut bytes = Vec::with_capacity(count * 2);
            for i in 0..count {
                let sample = if i < self.tail.len() {
                    let weight = i as f32 / OVERLAP as f32;
                    self.tail[i] * (1.0 - weight) + grain(i) * weight
                } else {
                    grain(i)
                };
                bytes.extend_from_slice(
                    &(sample.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16).to_le_bytes(),
                );
            }
            self.tail = (HOP..HOP + OVERLAP).map(grain).collect();
            self.emitted += count;
            output.push(bytes);
            self.next += HOP as f64 * self.speed;
            let discard = (self.next as usize)
                .saturating_sub(SEARCH)
                .saturating_sub(self.base)
                .min(self.input.len());
            self.input.drain(..discard);
            self.base += discard;
        }
        output
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn adjust(speed: f64, input: &[u8], packet: usize) -> Vec<i16> {
        let mut tempo = SpeechTempo::new(speed).unwrap();
        let mut bytes = Vec::new();
        for part in input.chunks(packet) {
            for out in tempo.push(part).unwrap() {
                bytes.extend(out);
            }
        }
        for out in tempo.finish() {
            bytes.extend(out);
        }
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect()
    }
    #[test]
    fn streamed_tempo_changes_duration_without_changing_pitch() {
        let input: Vec<u8> = (0..48000)
            .flat_map(|i| {
                ((i as f64 * 440.0 * std::f64::consts::TAU / 24000.0)
                    .sin()
                    .mul_add(6000.0, 0.0) as i16)
                    .to_le_bytes()
            })
            .collect();
        for speed in [0.7, 1.0, 1.2, 1.4, 1.6] {
            let output = adjust(speed, &input, 4096);
            assert_eq!(output.len(), (48000.0 / speed).round() as usize);
            let crossings = output[2400..output.len() - 2400]
                .windows(2)
                .filter(|v| v[0] <= 0 && v[1] > 0)
                .count();
            let hz = crossings as f64 * 24000.0 / (output.len() - 4800) as f64;
            assert!((hz - 440.0).abs() < 4.0, "speed={speed}, hz={hz}");
            assert!(output.iter().all(|s| s.unsigned_abs() <= 6000));
        }
        assert_eq!(
            adjust(1.0, &input, 4096)
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<_>>(),
            input
        );
        assert_eq!(adjust(1.4, &input, 4096), adjust(1.4, &input, 512));
    }
    #[test]
    fn short_tail_silence_and_invalid_packets_are_bounded() {
        assert!(SpeechTempo::new(f64::NAN).is_err());
        assert!(SpeechTempo::new(1.4).unwrap().push(&[0]).is_err());
        assert!(SpeechTempo::new(1.4).unwrap().push(&vec![0; 4098]).is_err());
        for count in [0, 1, 100, 480, 2000, 10000] {
            let output = adjust(1.4, &vec![0; count * 2], 512);
            assert_eq!(output.len(), (count as f64 / 1.4).round() as usize);
            assert!(output.iter().all(|s| *s == 0));
        }
    }
}
