import { useEffect, useRef, useState } from "react";
import { Switch } from "../../components/Switch";
import { I18N } from "../../lib/i18n";
import { outputCopy, outputError } from "../../lib/outputCopy";
import { microphoneCopy, outputCancel, outputCatalog, outputDevices, outputGet, outputPreview, outputSave, type CatalogItem, type OutputSnapshot, type TranslationOutput } from "../../lib/outputIpc";
import { hasConfirmedMicrophoneTranslation } from "../../lib/microphoneTranslation";
import { useStore } from "../../lib/store";
import { InlineFeedback, SettingsRow, SettingsSection, SettingsSelect } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

export function TranslationOutputSettings() {
  const session = useStore(state => state.session);
  const [saved, setSaved] = useState<OutputSnapshot | null>(null);
  const [draft, setDraft] = useState<TranslationOutput | null>(null);
  const [devices, setDevices] = useState<{ id: string; name: string }[]>([]);
  const [models, setModels] = useState<CatalogItem[]>([]);
  const [voices, setVoices] = useState<CatalogItem[]>([]);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false); const inFlight = useRef(false);
  const { beginToast } = useSettingsToast(); const c = outputCopy();
  useEffect(() => {
    mounted.current = true;
    void Promise.all([outputGet(), outputDevices()]).then(([snapshot, devices]) => {
      if (!mounted.current || !snapshot) return;
      setSaved(snapshot); setDraft(snapshot.settings); setDevices(devices);
    }).catch(() => { if (mounted.current) setError(outputCopy().error); });
    return () => { mounted.current = false; };
  }, []);
  const dirty = !!draft && (JSON.stringify(draft) !== JSON.stringify(saved?.settings) || !!key);
  const run = async (action: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true); setError(null);
    const notify = beginToast();
    try { await action(); }
    catch (error) { if (mounted.current) { setError(outputError(error)); notify(outputError(error), true); } }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  };
  const patch = (value: Partial<TranslationOutput>) => setDraft(current => current && ({ ...current, ...value }));
  const save = () => run(async () => {
    if (!draft) return;
    const result = await outputSave(draft, key || undefined);
    if (mounted.current) { setSaved(result); setDraft(result.settings); setKey(""); beginToast()(c.saved); }
  });
  const catalog = (isVoice: boolean) => run(async () => {
    const list = await outputCatalog(isVoice);
    if (mounted.current) { if (isVoice) setVoices(list); else setModels(list); }
  });
  const input = (label: string, field: "shortcut" | "endpoint" | "model" | "voice", placeholder?: string) => <input className="output-text-input" aria-label={label} dir="ltr" autoComplete="off" spellCheck={false} disabled={busy} value={draft?.[field] ?? ""} placeholder={placeholder} onChange={e => patch({ [field]: e.target.value })} />;
  return <SettingsSection id="translation-output" title={c.title}>
    {!draft ? error ? <InlineFeedback tone="error">{error}</InlineFeedback> : <InlineFeedback tone="info">{c.pending}</InlineFeedback> : <>
      <SettingsRow label={c.shortcut} description={c.shortcutHelp}>{input(c.shortcut, "shortcut", "Ctrl+Shift+F8")}</SettingsRow>
      <SettingsRow label={c.action}><SettingsSelect label={c.action} value={draft.shortcutAction} disabled={busy} options={[{ value: "insert", label: c.insert }, { value: "copy", label: c.copy }]} onChange={value => patch({ shortcutAction: value === "copy" ? "copy" : "insert" })} /></SettingsRow>
      <SettingsRow label={c.autoCopy} description={c.autoCopyHelp}><Switch aria-label={c.autoCopy} checked={draft.autoCopy} disabled={busy} onChange={autoCopy => patch({ autoCopy })} /></SettingsRow>
      <SettingsRow label={c.copy}><button className="settings-button settings-button--compact" disabled={busy || !hasConfirmedMicrophoneTranslation(session)} onClick={() => void run(async () => { await microphoneCopy(); beginToast()(c.done); })}>{c.copy}</button></SettingsRow>
      <div className="settings-divider" />
      <SettingsRow label={c.mic}><Switch aria-label={c.mic} checked={draft.speakMicrophone} disabled={busy} onChange={speakMicrophone => patch({ speakMicrophone })} /></SettingsRow>
      <SettingsRow label={c.system}><Switch aria-label={c.system} checked={draft.speakSystem} disabled={busy} onChange={speakSystem => patch({ speakSystem })} /></SettingsRow>
      <SettingsRow label={c.provider}><SettingsSelect label={c.provider} value={draft.speechProvider} disabled={busy} options={[{ value: "elevenLabs", label: "ElevenLabs" }, { value: "openAICompatible", label: "API سازگار با OpenAI" }]} onChange={value => { patch({ speechProvider: value === "elevenLabs" ? "elevenLabs" : "openAICompatible", model: "", voice: "", speakMicrophone: false, speakSystem: false }); setModels([]); setVoices([]); setKey(""); }} /></SettingsRow>
      {draft.speechProvider === "openAICompatible" && <SettingsRow label={c.endpoint}>{input(c.endpoint, "endpoint")}</SettingsRow>}
      <SettingsRow label={c.key}><input className="output-text-input" aria-label={c.key} dir="ltr" type="password" autoComplete="new-password" disabled={busy} value={key} placeholder={saved?.keyPresent && draft.speechProvider === saved.settings.speechProvider ? c.savedKey : I18N.settings.apiKeyPlaceholder} onChange={e => setKey(e.target.value)} /></SettingsRow>
      <SettingsRow label={c.model}>{input(c.model, "model")}</SettingsRow>
      {models.length > 0 && <SettingsRow label={c.catalog}><SettingsSelect label={c.catalog} value={draft.model} disabled={busy} options={models.map(m => ({ value: m.id, label: m.name, disabled: !m.compatible }))} onChange={model => patch({ model })} /></SettingsRow>}
      <SettingsRow label={c.voice} description={draft.speechProvider === "openAICompatible" ? c.manualVoice : undefined}>{input(c.voice, "voice")}</SettingsRow>
      {voices.length > 0 && <SettingsRow label={c.voice}><SettingsSelect label={c.voice} value={draft.voice} disabled={busy} options={voices.map(m => ({ value: m.id, label: m.name }))} onChange={voice => patch({ voice })} /></SettingsRow>}
      {(["speed", ...(draft.speechProvider === "elevenLabs" ? ["stability", "similarity"] : [])] as ("speed" | "stability" | "similarity")[]).map(field => <SettingsRow key={field} label={c[field]}><input className="output-number-input" aria-label={c[field]} dir="ltr" type="number" min={field === "speed" ? 0.7 : 0} max={field === "speed" ? 1.2 : 1} step="0.05" disabled={busy} value={draft[field]} onChange={e => { const value = e.target.valueAsNumber; if (Number.isFinite(value)) patch({ [field]: value }); }} /></SettingsRow>)}
      <SettingsRow label={c.device} description={c.deviceHelp}><SettingsSelect label={c.device} value={draft.device} disabled={busy} options={[{ value: "", label: c.disabledDevice }, ...(!devices.some(d => d.id === draft.device) && draft.device ? [{ value: draft.device, label: draft.device, disabled: true }] : []), ...devices.map(d => ({ value: d.id, label: d.name }))]} onChange={device => patch({ device })} /></SettingsRow>
      <div className="output-actions">
        <button className="settings-button settings-button--compact" disabled={busy} onClick={() => void run(async () => { const value = await outputDevices(); if (mounted.current) setDevices(value); })}>{c.refresh}</button>
        <button className="settings-button settings-button--compact" disabled={busy || dirty || !saved?.keyPresent} title={dirty ? c.saveFirst : undefined} onClick={() => void catalog(false)}>{c.fetchModels}</button>
        <button className="settings-button settings-button--compact" disabled={busy || dirty || !saved?.keyPresent || draft.speechProvider !== "elevenLabs"} title={dirty ? c.saveFirst : undefined} onClick={() => void catalog(true)}>{c.fetchVoices}</button>
        <button className="settings-button settings-button--compact" disabled={busy || dirty || session.isActive || !draft.device || !draft.model || !draft.voice || !saved?.keyPresent} onClick={() => void run(async () => { await outputPreview(); beginToast()(c.done); })}>{c.preview}</button>
        <button className="settings-button settings-button--compact" onClick={() => void outputCancel().catch(error => beginToast()(outputError(error), true))}>{c.cancel}</button>
        <button className="settings-button settings-button--primary settings-button--compact" disabled={busy || !dirty} onClick={() => void save()}>{c.save}</button>
      </div>
      {busy && <InlineFeedback tone="info">{c.pending}</InlineFeedback>}
      {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
    </>}
  </SettingsSection>;
}
