import { useEffect, useRef, useState } from "react";
import { Switch } from "../../components/Switch";
import { I18N } from "../../lib/i18n";
import { outputCopy, outputError } from "../../lib/outputCopy";
import { microphoneCopy, outputCancel, outputCatalog, outputDevices, outputGet, outputPreview, outputSave, type CatalogItem, type OutputSnapshot, type TranslationOutput, type SpeechSettings } from "../../lib/outputIpc";
import { hasConfirmedMicrophoneTranslation } from "../../lib/microphoneTranslation";
import { useStore } from "../../lib/store";
import { InlineFeedback, SettingsRow, SettingsSection, SettingsSelect } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";
import { VirtualMicrophoneSetup } from "./VirtualMicrophoneSetup";

function readerSettings(output: TranslationOutput): SpeechSettings {
  const { speechProvider, endpoint, model, voice, speed, stability, similarity, device } = output;
  return { speechProvider, endpoint, model, voice, speed, stability, similarity, device };
}

export function TranslationOutputSettings() {
  const session = useStore(state => state.session);
  const profileId = useStore(state => state.settings.activeProfileId);
  const [saved, setSaved] = useState<OutputSnapshot | null>(null);
  const [draft, setDraft] = useState<TranslationOutput | null>(null);
  const [devices, setDevices] = useState<{ id: string; name: string }[]>([]);
  const [models, setModels] = useState<CatalogItem[]>([]);
  const [voices, setVoices] = useState<CatalogItem[]>([]);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [incoming, setIncoming] = useState(true);
  const mounted = useRef(false); const inFlight = useRef(false);
  const { beginToast } = useSettingsToast(); const c = outputCopy();
  useEffect(() => {
    let current = true;
    mounted.current = true;
    void Promise.all([outputGet(), outputDevices()]).then(([snapshot, devices]) => {
      if (!current || !snapshot) return;
      setSaved(snapshot); setDraft(draft => draft ?? snapshot.settings); setDevices(devices); setModels([]); setVoices([]);
    }).catch(() => { if (mounted.current) setError(outputCopy().error); });
    return () => { current = false; mounted.current = false; };
  }, [profileId]);
  const dirty = !!draft && (JSON.stringify(draft) !== JSON.stringify(saved?.settings) || !!key);
  const speech = draft && (incoming ? draft.systemSpeech ?? draft : draft);
  const keyPresent = incoming ? saved?.systemKeyPresent ?? saved?.keyPresent : saved?.keyPresent;
  const savedSpeech = saved && (incoming ? saved.settings.systemSpeech ?? saved.settings : saved.settings);
  const catalogDirty = !!key || speech?.speechProvider !== savedSpeech?.speechProvider || speech?.endpoint !== savedSpeech?.endpoint;
  const run = async (action: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true); setError(null);
    const notify = beginToast();
    try { await action(); }
    catch (error) { if (mounted.current) { setError(outputError(error)); notify(outputError(error), true); } }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  };
  const patch = (value: Partial<TranslationOutput>) => setDraft(current => current && ({ ...current, ...value }));
  const patchSpeech = (value: Partial<SpeechSettings>) => setDraft(current => current && (incoming
    ? { ...current, systemSpeech: { ...(current.systemSpeech ?? readerSettings(current)), ...value } }
    : { ...current, ...value }));
  const save = () => run(async () => {
    if (!draft) return;
    const result = await outputSave(draft, key || undefined, false, incoming);
    if (mounted.current) { setSaved(result); setDraft(result.settings); setKey(""); beginToast()(c.saved); }
  });
  const catalog = (isVoice: boolean) => run(async () => {
    const list = await outputCatalog(isVoice, incoming);
    if (mounted.current) { if (isVoice) setVoices(list); else setModels(list); }
  });
  const input = (label: string, field: "shortcut" | "endpoint" | "model" | "voice", placeholder?: string) => <input className="output-text-input" aria-label={label} dir="ltr" autoComplete="off" spellCheck={false} disabled={busy} value={(field === "shortcut" ? draft?.shortcut : speech?.[field]) ?? ""} placeholder={placeholder} onChange={e => field === "shortcut" ? patch({ shortcut: e.target.value }) : patchSpeech({ [field]: e.target.value })} />;
  return <SettingsSection id="translation-output" title={c.title}>
    {!draft ? error ? <InlineFeedback tone="error">{error}</InlineFeedback> : <InlineFeedback tone="info">{c.pending}</InlineFeedback> : <>
      <h3>{c.speech}</h3>
      <SettingsRow label={c.lane} description={c.independentHelp}><SettingsSelect label={c.lane} value={incoming ? "system" : "microphone"} disabled={busy || dirty} options={[{ value: "system", label: c.incomingLane }, { value: "microphone", label: c.microphoneLane }]} onChange={value => { setIncoming(value === "system"); setModels([]); setVoices([]); setKey(""); }} /></SettingsRow>
      {incoming ? <SettingsRow label={c.system}><Switch aria-label={c.system} checked={draft.speakSystem} disabled={busy || session.isActive} onChange={speakSystem => patch({ speakSystem })} /></SettingsRow> : <SettingsRow label={c.mic}><Switch aria-label={c.mic} checked={draft.speakMicrophone} disabled={busy || session.isActive} onChange={speakMicrophone => patch({ speakMicrophone })} /></SettingsRow>}
      {incoming && <>
        <SettingsRow label={c.duck}><Switch aria-label={c.duck} checked={draft.duckSystemAudio ?? true} disabled={busy || session.isActive} onChange={duckSystemAudio => patch({ duckSystemAudio })} /></SettingsRow>
        <SettingsRow label={c.originalVolume}><input className="output-number-input" aria-label={c.originalVolume} dir="ltr" type="number" min={5} max={100} step={5} disabled={busy || session.isActive} value={Math.round((draft.originalVolume ?? 0.2) * 100)} onChange={e => { if (Number.isFinite(e.target.valueAsNumber)) patch({ originalVolume: e.target.valueAsNumber / 100 }); }} /></SettingsRow>
      </>}
      <SettingsRow label={c.provider}><SettingsSelect label={c.provider} value={speech!.speechProvider} disabled={busy || session.isActive} options={[{ value: "elevenLabs", label: "ElevenLabs" }, { value: "mainService", label: saved?.mainService ? `${c.mainService} — ${saved.mainService}` : c.mainService }, { value: "openAICompatible", label: "OpenAI-compatible API" }]} onChange={value => { patchSpeech({ speechProvider: value as SpeechSettings["speechProvider"], model: "", voice: "" }); patch(incoming ? { speakSystem: false } : { speakMicrophone: false }); setModels([]); setVoices([]); setKey(""); }} /></SettingsRow>
      {speech!.speechProvider === "openAICompatible" && <SettingsRow label={c.endpoint}>{input(c.endpoint, "endpoint")}</SettingsRow>}
      {speech!.speechProvider === "mainService" ? <SettingsRow label={c.mainService} description={c.mainHelp}><bdi>{saved?.mainService ?? c.notSupported}</bdi></SettingsRow> : <SettingsRow label={c.key}><input className="output-text-input" aria-label={c.key} dir="ltr" type="password" autoComplete="new-password" disabled={busy} value={key} placeholder={keyPresent ? c.savedKey : I18N.settings.apiKeyPlaceholder} onChange={e => setKey(e.target.value)} /></SettingsRow>}
      <SettingsRow label={c.model}>{input(c.model, "model")}</SettingsRow>
      {models.length > 0 && <SettingsRow label={c.catalog}><SettingsSelect label={c.catalog} value={speech!.model} disabled={busy} options={models.map(m => ({ value: m.id, label: m.name, disabled: !m.compatible }))} onChange={model => patchSpeech({ model })} /></SettingsRow>}
      <SettingsRow label={c.voice} description={speech!.speechProvider === "openAICompatible" ? c.manualVoice : undefined}>{input(c.voice, "voice")}</SettingsRow>
      {voices.length > 0 && <SettingsRow label={c.voice}><SettingsSelect label={c.voice} value={speech!.voice} disabled={busy} options={voices.map(m => ({ value: m.id, label: m.name }))} onChange={voice => patchSpeech({ voice })} /></SettingsRow>}
      {(speech!.speechProvider === "mainService" ? [] : (["speed", ...(speech!.speechProvider === "elevenLabs" ? ["stability", "similarity"] : [])] as ("speed" | "stability" | "similarity")[])).map(field => <SettingsRow key={field} label={c[field]}><input className="output-number-input" aria-label={c[field]} dir="ltr" type="number" min={field === "speed" ? 0.7 : 0} max={field === "speed" ? 1.2 : 1} step="0.05" disabled={busy} value={speech![field]} onChange={e => { const value = e.target.valueAsNumber; if (Number.isFinite(value)) patchSpeech({ [field]: value }); }} /></SettingsRow>)}
      <SettingsRow label={c.device} description={incoming ? c.incomingDeviceHelp : c.deviceHelp}><SettingsSelect label={c.device} value={speech!.device} disabled={busy} options={[{ value: "", label: c.disabledDevice }, ...(!devices.some(d => d.id === speech!.device) && speech!.device ? [{ value: speech!.device, label: speech!.device, disabled: true }] : []), ...devices.map(d => ({ value: d.id, label: d.name }))]} onChange={device => patchSpeech({ device })} /></SettingsRow>
      {!incoming && <VirtualMicrophoneSetup disabled={busy || dirty || session.isActive} run={run} onConfigured={async () => {
        const [snapshot, nextDevices] = await Promise.all([outputGet(), outputDevices()]);
        if (mounted.current) { setSaved(snapshot); setDraft(snapshot.settings); setDevices(nextDevices); }
      }} />}
      <div className="settings-divider" />
      <h3>{c.microphoneText}</h3>
      <SettingsRow label={c.shortcut} description={c.shortcutHelp}>{input(c.shortcut, "shortcut", "Ctrl+Shift+F8")}</SettingsRow>
      <SettingsRow label={c.action}><SettingsSelect label={c.action} value={draft.shortcutAction} disabled={busy} options={[{ value: "insert", label: c.insert }, { value: "copy", label: c.copy }]} onChange={value => patch({ shortcutAction: value === "copy" ? "copy" : "insert" })} /></SettingsRow>
      <SettingsRow label={c.autoCopy} description={c.autoCopyHelp}><Switch aria-label={c.autoCopy} checked={draft.autoCopy} disabled={busy} onChange={autoCopy => patch({ autoCopy })} /></SettingsRow>
      <SettingsRow label={c.copy}><button className="settings-button settings-button--compact" disabled={busy || !hasConfirmedMicrophoneTranslation(session)} onClick={() => void run(async () => { await microphoneCopy(); beginToast()(c.done); })}>{c.copy}</button></SettingsRow>
      <div className="output-actions">
        <button className="settings-button settings-button--compact" disabled={busy} onClick={() => void run(async () => { const value = await outputDevices(); if (mounted.current) setDevices(value); })}>{c.refresh}</button>
        <button className="settings-button settings-button--compact" disabled={busy || catalogDirty || !keyPresent} title={catalogDirty ? c.saveFirst : undefined} onClick={() => void catalog(false)}>{c.fetchModels}</button>
        <button className="settings-button settings-button--compact" disabled={busy || catalogDirty || !keyPresent || speech!.speechProvider === "openAICompatible"} title={catalogDirty ? c.saveFirst : undefined} onClick={() => void catalog(true)}>{c.fetchVoices}</button>
        <button className="settings-button settings-button--compact" disabled={busy || dirty || session.isActive || !speech!.device || !speech!.model || !speech!.voice || !keyPresent} onClick={() => void run(async () => { await outputPreview(incoming); beginToast()(c.done); })}>{c.preview}</button>
        <button className="settings-button settings-button--compact" onClick={() => void outputCancel().catch(error => beginToast()(outputError(error), true))}>{c.cancel}</button>
        <button className="settings-button settings-button--primary settings-button--compact" disabled={busy || !dirty} onClick={() => void save()}>{c.save}</button>
      </div>
      {busy && <InlineFeedback tone="info">{c.pending}</InlineFeedback>}
      {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
    </>}
  </SettingsSection>;
}
