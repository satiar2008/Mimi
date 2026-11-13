//! Bounded, per-source speech commitment. Clipboard/insertion policy stays final-only.
use crate::core::{audio_input::AudioSource, models::SubtitleSnapshot};
use std::collections::VecDeque;

#[derive(Default)]
struct Lane {
    entries: VecDeque<Entry>,
    last_final: Option<(Option<String>, u64)>,
}
#[derive(Default)]
struct Entry {
    key: String,
    text: String,
    ages: Vec<u64>,
    committed: String,
    finished: bool,
    revised: bool,
}
#[derive(Default)]
pub struct SpeechCursor {
    generation: u64,
    lanes: [Lane; 2],
}
pub struct SpeechPhrase {
    pub source: AudioSource,
    pub text: String,
    pub preview: bool,
}
impl SpeechCursor {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn observe(
        &mut self,
        snapshot: &SubtitleSnapshot,
        generation: u64,
        now_ms: u64,
        enabled: [bool; 2],
    ) -> Vec<SpeechPhrase> {
        if self.generation != generation {
            self.clear();
            self.generation = generation;
        }
        let mut phrases = Vec::new();
        for track in snapshot.tracks.iter().take(2) {
            let slot = usize::from(track.audio_source == AudioSource::Microphone);
            if !enabled[slot] {
                continue;
            }
            let lane = &mut self.lanes[slot];
            let final_pair = track
                .display_pair
                .as_ref()
                .filter(|_| track.display_pair_final);
            if let (Some(pair), Some(created)) = (final_pair, track.display_pair_created_at_ms) {
                let identity = (pair.utterance_id.clone(), created);
                if created >= generation && lane.last_final.as_ref() != Some(&identity) {
                    let key = key(
                        pair.utterance_id.as_deref(),
                        lane.last_final.as_ref().map_or(0, |v| v.1),
                    );
                    for text in lane.accept(key, &pair.translation, true, now_ms) {
                        phrases.push(SpeechPhrase {
                            source: track.audio_source,
                            text,
                            preview: false,
                        });
                    }
                    lane.last_final = Some(identity);
                }
            }
            let preview = track
                .realtime_preview
                .as_ref()
                .map(|p| {
                    (
                        p.translation.utterance_id.as_deref(),
                        p.translation.text.as_str(),
                    )
                })
                .filter(|(_, text)| !text.trim().is_empty())
                .or_else(|| {
                    track
                        .preview_pair
                        .as_ref()
                        .map(|p| (p.utterance_id.as_deref(), p.translation.as_str()))
                })
                .or_else(|| {
                    track
                        .display_pair
                        .as_ref()
                        .filter(|_| !track.display_pair_final)
                        .map(|p| (p.utterance_id.as_deref(), p.translation.as_str()))
                });
            if let Some((id, text)) = preview {
                if final_pair
                    .is_some_and(|p| p.translation == text && p.utterance_id.as_deref() == id)
                {
                    continue;
                }
                let key = key(id, lane.last_final.as_ref().map_or(0, |v| v.1));
                for text in lane.accept(key, text, false, now_ms) {
                    phrases.push(SpeechPhrase {
                        source: track.audio_source,
                        text,
                        preview: true,
                    });
                }
            }
        }
        phrases
    }
}
fn key(id: Option<&str>, epoch: u64) -> String {
    id.map_or_else(|| format!("anonymous:{epoch}"), |id| format!("id:{id}"))
}
impl Lane {
    fn accept(&mut self, key: String, text: &str, final_text: bool, now: u64) -> Vec<String> {
        let Some(text) = crate::core::microphone_input::insertion_text(text) else {
            return Vec::new();
        };
        let index = if let Some(index) = self.entries.iter().position(|v| v.key == key) {
            index
        } else {
            if self.entries.len() == 8 {
                self.entries.pop_front();
            }
            self.entries.push_back(Entry {
                key,
                ..Default::default()
            });
            self.entries.len() - 1
        };
        self.entries[index].accept(&text, final_text, now)
    }
}
impl Entry {
    fn accept(&mut self, text: &str, final_text: bool, now: u64) -> Vec<String> {
        if self.finished || self.revised {
            return Vec::new();
        }
        // Speech cannot be taken back. Never replay a rewritten spoken prefix.
        if !text.starts_with(&self.committed) {
            self.revised = true;
            return Vec::new();
        }
        let common = self
            .text
            .chars()
            .zip(text.chars())
            .take_while(|(a, b)| a == b)
            .count();
        self.ages.truncate(common);
        self.ages.resize(text.chars().count(), now);
        self.text = text.to_owned();
        let stable = if final_text {
            text.len()
        } else {
            text.char_indices()
                .zip(&self.ages)
                .take_while(|(_, age)| now.saturating_sub(**age) >= 350)
                .last()
                .map_or(0, |((index, c), _)| index + c.len_utf8())
        };
        let mut end = stable;
        if !final_text {
            // Keep the newest two words replaceable, and never send a half word.
            let starts: Vec<usize> = text
                .char_indices()
                .filter_map(|(i, c)| {
                    (!c.is_whitespace()
                        && (i == 0
                            || text[..i]
                                .chars()
                                .next_back()
                                .is_some_and(char::is_whitespace)))
                    .then_some(i)
                })
                .collect();
            if starts.len() > 2 {
                end = end.min(starts[starts.len() - 2]);
            } else {
                end = 0;
            }
            end = text[..end]
                .char_indices()
                .rfind(|(_, c)| c.is_whitespace() || punctuation(*c))
                .map_or(0, |(i, c)| i + c.len_utf8());
        }
        let mut result = Vec::new();
        while self.committed.len() < end && result.len() < 8 {
            let rest = &text[self.committed.len()..end];
            let Some(length) = phrase_end(rest, final_text) else {
                break;
            };
            let phrase = rest[..length].trim();
            if !phrase.is_empty() {
                result.push(phrase.to_owned());
            }
            self.committed.push_str(&rest[..length]);
        }
        if final_text && self.committed.len() == text.len() {
            self.finished = true;
        }
        result
    }
}
fn punctuation(c: char) -> bool {
    matches!(
        c,
        '.' | '!' | '?' | '؟' | '،' | ',' | ';' | '؛' | '。' | '！' | '？' | ':'
    )
}
fn phrase_end(text: &str, final_text: bool) -> Option<usize> {
    if final_text {
        if text.chars().count() <= 4000 {
            return Some(text.len());
        }
        return text
            .char_indices()
            .take(4000)
            .filter(|(_, c)| c.is_whitespace())
            .last()
            .map(|(i, c)| i + c.len_utf8());
    }
    let mut words = 0;
    let mut inside = false;
    let mut boundary = None;
    for (count, (i, c)) in text.char_indices().enumerate() {
        if c.is_whitespace() {
            if inside {
                words += 1;
                inside = false;
            }
            boundary = Some(i + c.len_utf8());
            if words >= 8 {
                return boundary;
            }
        } else {
            inside = true;
        }
        if punctuation(c)
            && count >= 6
            && text[i + c.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        {
            return Some(i + c.len_utf8());
        }
        if count >= 180 {
            return boundary;
        }
    }
    if words >= 4 {
        boundary
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(final_text: bool, id: Option<&str>) -> SubtitleSnapshot {
        serde_json::from_value(serde_json::json!({"tracks":[{"audioSource":"system","source":{"text":"source","isFinal":false},"translation":{"text":"","isFinal":false},"history":[],"previewPair":null,"displayPair":{"source":"source","translation":"Hello Mimi this is a streaming translation test for today","utteranceId":id},"displayPairFinal":final_text,"displayPairCreatedAt":10,"detectedLanguage":null,"isTranslationPending":false,"isTranslationPreviewPending":false,"isTranslationTimedOut":false,"translationRecovery":null}],"source":{"text":"","isFinal":false},"translation":{"text":"","isFinal":false},"history":[],"previewPair":null})).unwrap()
    }
    #[test]
    fn two_input_lanes_commit_early_and_do_not_repeat_their_finals_after_resume() {
        let mut s = snapshot(false, Some("same-provider-id"));
        let mut mic = s.tracks[0].clone();
        mic.audio_source = AudioSource::Microphone;
        s.tracks.push(mic);
        let mut cursor = SpeechCursor::default();
        assert!(cursor.observe(&s, 1, 0, [true, true]).is_empty());
        let early = cursor.observe(&s, 1, 400, [true, true]);
        assert_eq!(early.len(), 2);
        assert_ne!(early[0].source, early[1].source);
        for track in &mut s.tracks {
            track.display_pair_final = true;
        }
        let tail = cursor.observe(&s, 1, 450, [true, true]);
        assert_eq!(tail.len(), 2);
        assert!(tail.iter().all(|p| !p.text.starts_with("Hello")));
        assert!(cursor.observe(&s, 1, 1200, [true, true]).is_empty());
        assert!(cursor.observe(&s, 20, 1600, [true, true]).is_empty());
    }
    #[test]
    fn anonymous_finals_and_disabled_outputs_do_not_duplicate_or_send_drafts() {
        let mut cursor = SpeechCursor::default();
        let s = snapshot(false, None);
        assert!(cursor.observe(&s, 1, 0, [false, false]).is_empty());
        assert!(cursor.observe(&s, 1, 400, [false, false]).is_empty());
        assert!(cursor.observe(&s, 1, 500, [true, false]).is_empty());
        assert!(!cursor.observe(&s, 1, 900, [true, false]).is_empty());
        let s = snapshot(true, None);
        assert!(!cursor.observe(&s, 1, 1000, [true, false]).is_empty());
        assert!(cursor.observe(&s, 1, 1100, [true, false]).is_empty());
    }
    #[test]
    fn continuous_growth_speaks_before_final_and_final_does_not_replay() {
        let mut entry = Entry::default();
        let first = "سلام این یک آزمایش ترجمه زنده برای جلسه امروز است";
        assert!(entry.accept(first, false, 0).is_empty());
        let chunks = entry.accept(&format!("{first} و صحبت ادامه دارد"), false, 400);
        assert!(!chunks.is_empty());
        let spoken = chunks.join(" ");
        let rest = entry
            .accept(&format!("{first} و صحبت ادامه دارد"), true, 500)
            .join(" ");
        assert!(!rest.starts_with(&spoken));
        assert!(entry
            .accept(&format!("{first} و صحبت ادامه دارد"), true, 600)
            .is_empty());
    }
    #[test]
    fn rewritten_unspoken_tail_is_read_but_spoken_prefix_is_not_repeated() {
        let mut entry = Entry::default();
        let text = "Hello Mimi this is a live translation test for today";
        entry.accept(text, false, 0);
        assert!(!entry.accept(text, false, 400).is_empty());
        assert!(!entry
            .accept(&format!("{} tomorrow", entry.committed), true, 500)
            .is_empty());
        let mut entry = Entry::default();
        entry.accept(text, false, 0);
        entry.accept(text, false, 400);
        assert!(entry
            .accept("Changed beginning of the whole sentence", true, 500)
            .is_empty());
    }
    #[test]
    fn unicode_and_long_finals_remain_bounded_without_breaking_utf8() {
        let mut entry = Entry::default();
        let chunks = entry.accept(&"سلام Mimi 👋 جلسهٔ ۱۲:۳۰ ادامه دارد. ".repeat(30), true, 0);
        assert!(!chunks.is_empty() && chunks.len() <= 8);
        assert!(chunks.iter().all(|v| v.chars().count() <= 4000));
        assert!(Entry::default()
            .accept(&"x".repeat(4097), false, 0)
            .is_empty());
    }
}
