import { effectiveUiLanguage } from "./i18n";
import type { AudioSource, MicrophoneTranslation, SessionStateEvent, SettingsSnapshot, TargetLanguage } from "./types";

export const DEFAULT_MICROPHONE_TRANSLATION: MicrophoneTranslation = {
  enabled: false, sourceLanguage: "auto", targetLanguage: null,
};

const en = {
  title: "Microphone translation", independent: "Separate microphone languages",
  help: "Translate your microphone independently from incoming audio. Stop subtitles before changing these languages.",
  source: "Microphone speech language", target: "Microphone translation language", incoming: "Same as incoming speech",
  unknown: "Choose the incoming speech language, or select a specific microphone translation language.",
  unsupported: "The selected service does not support this microphone language pair. Choose compatible languages or another service.",
  stopped: "Stop subtitles to change microphone languages.",
  insertAction: "Insert text", insert: "Insert microphone translation", insertHelp: "After clicking, focus a writable text field within 3 seconds. Inserts only the current confirmed microphone translation, once. Passwords and unsupported editors are excluded. Line breaks become spaces; Enter is never pressed.",
  pending: "Focus the destination text field now (3 seconds)…", cancel: "Cancel insertion",
  done: "Microphone translation inserted.", failed: "Could not insert. Focus a supported, writable text field and try again.",
  noFinal: "Wait for a confirmed microphone translation.", duplicate: "This microphone translation has already been inserted.",
  changed: "Insertion cancelled because the session or microphone translation changed.", cancelled: "Insertion cancelled.",
  incomplete: "Windows did not accept all of the text. Check the destination before continuing; this translation will not be inserted again.",
  saveFailed: "Could not save microphone languages.",
};
const fa: typeof en = {
  title: "ترجمهٔ میکروفون", independent: "زبان مستقل برای میکروفون",
  help: "گفتار میکروفون را مستقل از صدای دریافتی ترجمه کنید. برای تغییر زبان‌ها، ابتدا زیرنویس را متوقف کنید.",
  source: "زبان گفتار میکروفون", target: "زبان ترجمهٔ میکروفون", incoming: "هم‌زبان با گفتار دریافتی",
  unknown: "زبان گفتار دریافتی را مشخص کنید یا برای ترجمهٔ میکروفون زبان مقصد مشخصی انتخاب کنید.",
  unsupported: "سرویس انتخابی از این زبان‌های میکروفون پشتیبانی نمی‌کند. زبان‌های سازگار یا سرویس دیگری انتخاب کنید.",
  stopped: "برای تغییر زبان میکروفون، زیرنویس را متوقف کنید.",
  insertAction: "درج متن", insert: "درج ترجمهٔ میکروفون در محل تایپ", insertHelp: "پس از زدن دکمه، ظرف ۳ ثانیه کادر نوشتن مقصد را فعال کنید. فقط ترجمهٔ نهایی فعلی میکروفون، یک بار درج می‌شود. کادر رمز عبور و ویرایشگرهای پشتیبانی‌نشده پذیرفته نمی‌شوند. خط جدید به فاصله تبدیل می‌شود و کلید ارسال زده نمی‌شود.",
  pending: "اکنون کادر نوشتن مقصد را فعال کنید؛ ۳ ثانیه فرصت دارید…", cancel: "لغو درج متن",
  done: "ترجمهٔ میکروفون درج شد.", failed: "درج انجام نشد. یک کادر نوشتن قابل ویرایش و پشتیبانی‌شده را فعال کنید و دوباره تلاش کنید.",
  noFinal: "منتظر ترجمهٔ نهایی میکروفون بمانید.", duplicate: "این ترجمهٔ میکروفون قبلاً درج شده است.",
  changed: "به‌دلیل تغییر نشست یا ترجمهٔ میکروفون، درج متن لغو شد.", cancelled: "درج متن لغو شد.",
  incomplete: "ویندوز همهٔ متن را نپذیرفت. پیش از ادامه، متن مقصد را بررسی کنید؛ این ترجمه دوباره درج نخواهد شد.",
  saveFailed: "ذخیرهٔ زبان میکروفون انجام نشد.",
};

export function microphoneTranslationCopy() { return effectiveUiLanguage() === "fa-IR" ? fa : en; }
export function microphoneTranslationError(error: unknown): string | null {
  const label = error instanceof Error ? error.message : String(error);
  const copy = microphoneTranslationCopy();
  if (label === "microphone_translation_incoming_language_required") return copy.unknown;
  if (label === "microphone_translation_requires_stop") return copy.stopped;
  if (label === "microphone_input_no_final") return copy.noFinal;
  if (label === "microphone_input_duplicate") return copy.duplicate;
  if (label === "microphone_input_cancelled") return copy.cancelled;
  if (label === "microphone_input_changed") return copy.changed;
  if (label === "microphone_input_incomplete") return copy.incomplete;
  if (label.startsWith("microphone_input_")) return copy.failed;
  if (label.startsWith("microphone_translation_")) return copy.saveFailed;
  return null;
}

export function incomingMicrophoneLanguageUnknown(settings: SettingsSnapshot) {
  const choice = settings.microphoneTranslation;
  return choice?.enabled === true && choice.targetLanguage === null && settings.sourceLanguage === "auto";
}

export function hasConfirmedMicrophoneTranslation(session: SessionStateEvent): boolean {
  return session.isActive && !!session.subtitles.tracks?.some(track => track.audioSource === "microphone"
    && track.displayPairFinal === true && track.displayPairCreatedAt !== undefined
    && !!track.displayPair?.translation.trim());
}

/** Presentation only; the native resolver validates the actual provider route. */
export function languageSettingsForAudioSource<T extends Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage" | "microphoneTranslation">>(settings: T, source: AudioSource): T {
  const mic = settings.microphoneTranslation;
  if (source !== "microphone" || !mic?.enabled) return settings;
  return { ...settings, sourceLanguage: mic.sourceLanguage,
    targetLanguage: mic.targetLanguage ?? (settings.sourceLanguage === "auto" ? settings.targetLanguage : settings.sourceLanguage as TargetLanguage) };
}
