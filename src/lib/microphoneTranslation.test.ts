import { afterEach, expect, it } from "vitest";
import { setStoredUiLanguage } from "./i18n";
import { useStore } from "./store";
import { mergeSettingsSnapshot } from "./settingsState";
import { hasConfirmedMicrophoneTranslation, incomingMicrophoneLanguageUnknown, microphoneTranslationError, languageSettingsForAudioSource } from "./microphoneTranslation";
import { languageStatus, visibleLiveSubtitle } from "../windows/overlay/overlayModel";

const initial = useStore.getState();
afterEach(() => { useStore.setState(initial, true); setStoredUiLanguage("system"); });

it("preserves the system route and does not restart or change selected inputs", () => {
  const current = { ...initial.settings, sourceLanguage: "en" as const, targetLanguage: "fa" as const };
  const choice = { enabled: true, sourceLanguage: "fa" as const, targetLanguage: null };
  const next = mergeSettingsSnapshot(current, { microphoneTranslation: choice });
  expect(next.microphoneTranslation).toEqual(choice);
  expect(next.sourceLanguage).toBe("en"); expect(next.targetLanguage).toBe("fa");
  expect(next.audioInput).toBe(current.audioInput);
  expect(incomingMicrophoneLanguageUnknown(next)).toBe(false);
  expect(incomingMicrophoneLanguageUnknown({ ...next, sourceLanguage: "auto" })).toBe(true);
  expect(incomingMicrophoneLanguageUnknown({ ...next, sourceLanguage: "auto", microphoneTranslation: { ...choice, targetLanguage: "de" } })).toBe(false);
});

it.each([false, true])("blocks microphone language edits during an active/paused session: %s", async isPaused => {
  useStore.setState({ session: { ...initial.session, isActive: !isPaused, isPaused } });
  await expect(useStore.getState().saveSettings({ microphoneTranslation: { enabled: true, sourceLanguage: "fa", targetLanguage: "en" } })).rejects.toThrow("microphone_translation_requires_stop");
  expect(useStore.getState().settings).toEqual(initial.settings);
});

it("never enables insertion for history, system audio or unconfirmed previews", () => {
  const track = { audioSource: "microphone" as const, source: { text: "گفتار", isFinal: true }, translation: { text: "Speech", isFinal: true },
    displayPair: { source: "گفتار", translation: "Speech" }, displayPairFinal: true, displayPairCreatedAt: 123,
    history: [], detectedLanguage: "fa", isTranslationPending: false, isTranslationTimedOut: false };
  const session = { ...initial.session, isActive: true, subtitles: { ...initial.session.subtitles, tracks: [track] } };
  expect(hasConfirmedMicrophoneTranslation(session)).toBe(true);
  expect(hasConfirmedMicrophoneTranslation({ ...session, isActive: false })).toBe(false);
  expect(hasConfirmedMicrophoneTranslation({ ...session, subtitles: { ...session.subtitles, tracks: [{ ...track, displayPairFinal: false }] } })).toBe(false);
  expect(hasConfirmedMicrophoneTranslation({ ...session, subtitles: { ...session.subtitles, tracks: [{ ...track, audioSource: "system" }] } })).toBe(false);
});

it("localizes sanitized failures without exposing native/provider text", () => {
  setStoredUiLanguage("fa-IR");
  expect(microphoneTranslationError("microphone_input_duplicate")).toContain("قبلاً");
  expect(microphoneTranslationError("microphone_translation_incoming_language_required")).toContain("مقصد");
  expect(microphoneTranslationError("secret provider message")).toBeNull();
});

it("renders the microphone translation when its source is the system translation target", () => {
  const settings = { ...initial.settings, sourceLanguage: "en" as const, targetLanguage: "fa" as const,
    microphoneTranslation: { enabled: true, sourceLanguage: "fa" as const, targetLanguage: null } };
  const mic = languageSettingsForAudioSource(settings, "microphone");
  expect(mic.sourceLanguage).toBe("fa"); expect(mic.targetLanguage).toBe("en");
  expect(languageSettingsForAudioSource(settings, "system")).toBe(settings);
  const subtitles = { ...initial.session.subtitles, source: { text: "سلام", isFinal: false }, translation: { text: "Hello", isFinal: false } };
  expect(visibleLiveSubtitle(subtitles, mic, "fa", false, false)?.text).toBe("Hello");
  const reverseOriginal = languageSettingsForAudioSource({ ...settings, targetLanguage: "original" as const }, "microphone");
  expect(reverseOriginal.targetLanguage).toBe("en");
});

it("shows the actual microphone-only language direction while retaining incoming direction for both", () => {
  setStoredUiLanguage("en");
  const settings = { ...initial.settings, sourceLanguage: "en" as const, targetLanguage: "fa" as const,
    microphoneTranslation: { enabled: true, sourceLanguage: "fa" as const, targetLanguage: null } };
  const mic = languageStatus({ ...settings, audioInput: "microphone" }, "fa");
  expect(mic).toMatchObject({ source: "Persian", target: "English" });
  const both = languageStatus({ ...settings, audioInput: "both" }, null);
  expect(both).toMatchObject({ source: "English", target: "Persian" });
});
