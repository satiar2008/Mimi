import { useEffect, useRef, useState } from "react";
import { LanguageSelect } from "../../components/LanguageSelect";
import { Switch } from "../../components/Switch";
import { activeServiceProfile, sourceLanguagesForSettings, targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { microphoneCancelInsertion, microphoneInsertTranslation } from "../../lib/ipc";
import { DEFAULT_MICROPHONE_TRANSLATION, hasConfirmedMicrophoneTranslation, incomingMicrophoneLanguageUnknown, microphoneTranslationCopy, microphoneTranslationError } from "../../lib/microphoneTranslation";
import { useStore } from "../../lib/store";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type MicrophoneTranslation, type TargetLanguage } from "../../lib/types";
import { InlineFeedback, SettingsRow, SettingsSection } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

export function MicrophoneTranslationSettings() {
  const settings = useStore(state => state.settings);
  const session = useStore(state => state.session);
  const saveSettings = useStore(state => state.saveSettings);
  const [busy, setBusy] = useState(false);
  const [inserting, setInserting] = useState(false);
  const result = useRef<{ text: string; failed: boolean } | null>(null);
  const inFlight = useRef(false);
  const insertion = useRef(false);
  const mounted = useRef(false);
  const { beginToast } = useSettingsToast();
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; if (insertion.current) void microphoneCancelInsertion().catch(() => {}); };
  }, []);
  useEffect(() => {
    const show = () => { if (document.hasFocus() && result.current) {
      const notice = result.current; result.current = null;
      beginToast()(notice.text, notice.failed);
    } };
    window.addEventListener("focus", show);
    return () => window.removeEventListener("focus", show);
  }, [beginToast]);
  const copy = microphoneTranslationCopy();
  if (!settings.microphoneInputAvailable) return null;
  const choice = settings.microphoneTranslation ?? DEFAULT_MICROPHONE_TRANSLATION;
  const locked = busy || session.isActive || session.isPaused || session.status.kind === "connecting" || session.status.kind === "stopping";
  const selectedTarget = choice.targetLanguage ?? (settings.sourceLanguage !== "auto" ? settings.sourceLanguage as TargetLanguage : settings.targetLanguage);
  const languageSettings = { ...settings, targetLanguage: selectedTarget, languageCapabilities: undefined };
  const sources = sourceLanguagesForSettings(languageSettings);
  const targets = targetLanguagesForSettings(languageSettings).filter(language => language !== "original");
  const save = async (next: MicrophoneTranslation) => {
    if (locked || inFlight.current) return;
    inFlight.current = true; setBusy(true);
    const notify = beginToast();
    try { await saveSettings({ microphoneTranslation: next }); }
    catch (error) { notify(microphoneTranslationError(error) ?? copy.saveFailed, true); }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  };
  const insert = async () => {
    if (insertion.current) return;
    insertion.current = true; setInserting(true); result.current = null;
    try {
      await microphoneInsertTranslation();
      if (mounted.current) result.current = { text: copy.done, failed: false };
    } catch (error) {
      if (mounted.current) result.current = { text: microphoneTranslationError(error) ?? copy.failed, failed: true };
    } finally {
      insertion.current = false;
      if (mounted.current) {
        setInserting(false);
        if (document.hasFocus() && result.current) {
          const notice = result.current; result.current = null;
          beginToast()(notice.text, notice.failed);
        }
      }
    }
  };
  return <SettingsSection id="microphone-translation" title={copy.title}>
    <SettingsRow label={copy.independent} description={copy.help} feedback={session.isActive && <InlineFeedback tone="info">{copy.stopped}</InlineFeedback>}>
      <Switch aria-label={copy.independent} checked={choice.enabled} disabled={locked || !activeServiceProfile(settings)} onChange={enabled => void save({ ...choice, enabled })} />
    </SettingsRow>
    {choice.enabled && <>
      <SettingsRow label={copy.source} feedback={!sources.includes(choice.sourceLanguage) && <InlineFeedback tone="error">{copy.unsupported}</InlineFeedback>}>
        <LanguageSelect label={copy.source} value={choice.sourceLanguage} valueLabel={SOURCE_LANGUAGE_DISPLAY_NAMES[choice.sourceLanguage]} disabled={locked} options={sources.map(value => ({ value, label: SOURCE_LANGUAGE_DISPLAY_NAMES[value] }))}
          onChange={value => { const language = sources.find(source => source === value); if (language) void save({ ...choice, sourceLanguage: language }); }} />
      </SettingsRow>
      <SettingsRow label={copy.target} feedback={incomingMicrophoneLanguageUnknown(settings) && <InlineFeedback tone="info">{copy.unknown}</InlineFeedback>}>
        <LanguageSelect label={copy.target} value={choice.targetLanguage ?? "incoming"} valueLabel={choice.targetLanguage ? TARGET_LANGUAGE_DISPLAY_NAMES[choice.targetLanguage] : copy.incoming} disabled={locked}
          options={[{ value: "incoming", label: copy.incoming }, ...targets.map(value => ({ value, label: TARGET_LANGUAGE_DISPLAY_NAMES[value] }))]}
          onChange={value => { const language = targets.find(target => target === value); if (value === "incoming" || language) void save({ ...choice, targetLanguage: language ?? null }); }} />
      </SettingsRow>
    </>}
    {settings.microphoneTextInsertionAvailable && <SettingsRow label={copy.insert} description={copy.insertHelp}
      feedback={inserting ? <InlineFeedback tone="info">{copy.pending}</InlineFeedback> : undefined}>
      <button type="button" className="settings-button settings-button--compact" disabled={!inserting && !hasConfirmedMicrophoneTranslation(session)}
        onClick={() => { if (inserting) void microphoneCancelInsertion().catch(() => {}); else void insert(); }}>{inserting ? copy.cancel : copy.insertAction}</button>
    </SettingsRow>}
  </SettingsSection>;
}
