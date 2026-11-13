import { useEffect, useRef, useState } from "react";
import type { ServiceProfile } from "../../lib/types";
import { outputCopy, outputError } from "../../lib/outputCopy";
import { providerModels, providerModelSave, type ModelView } from "../../lib/outputIpc";
import { InlineFeedback, SettingsRow, SettingsSelect } from "./SettingsPrimitives";
import { ServiceEditorModule } from "./ServiceEditorModule";
import { useSettingsToast } from "./useSettingsToast";

export function ProviderModels({ profile, disabled }: { profile: ServiceProfile; disabled: boolean }) {
  const [view, setView] = useState<ModelView | null>(null);
  const [model, setModel] = useState("");
  const [text, setText] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const epoch = useRef(0); const mounted = useRef(false); const pending = useRef(false);
  const { beginToast } = useSettingsToast(); const c = outputCopy();
  const hasTextCatalog = ["openAICompatible", "chatMock", "xAI"].includes(profile.textTranslation ?? "");
  const load = async (fetch: boolean, textRoute = text) => {
    const request = ++epoch.current; setBusy(true); setError(null);
    try { const result = await providerModels(profile.id, fetch, textRoute); if (mounted.current && request === epoch.current) { setView(result); setModel(result.current[textRoute ? 1 : 0] ?? ""); } }
    catch (error) { if (mounted.current && request === epoch.current) setError(outputError(error)); }
    finally { if (mounted.current && request === epoch.current) setBusy(false); }
  };
  useEffect(() => {
    mounted.current = true;
    const request = ++epoch.current;
    void providerModels(profile.id).then(result => { if (mounted.current && request === epoch.current) { setView(result); setModel(result.current[0] ?? ""); } }).catch(() => { /* Missing setup is already represented by the current-model placeholder. */ });
    const refresh = (event: Event) => {
      const detail = (event as CustomEvent<string | { profileId: string; stage: string }>).detail;
      const id = typeof detail === "string" ? detail : detail.profileId;
      if (id !== profile.id) return;
      const textRoute = typeof detail !== "string" && detail.stage === "text";
      setText(textRoute); void load(true, textRoute);
    };
    window.addEventListener("mimi-model-refresh", refresh);
    return () => { mounted.current = false; epoch.current += 1; window.removeEventListener("mimi-model-refresh", refresh); };
    // Each editor is keyed by profile ID; old network responses cannot replace a new editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profile.id, profile.speechTranslationMode, profile.credentialState]);
  const save = async () => {
    if (pending.current || disabled || busy) return;
    pending.current = true; setBusy(true); const notify = beginToast();
    try { await providerModelSave(profile.id, model.trim(), text); await load(false); notify(c.savedModel); }
    catch (error) { notify(outputError(error), true); }
    finally { pending.current = false; if (mounted.current) setBusy(false); }
  };
  return <ServiceEditorModule kind="models">
    <SettingsRow label={c.current} description={c.modelsHelp}><bdi dir="ltr" className="output-model-name">{view?.current.filter(Boolean).join(" · ") || c.unavailable}</bdi></SettingsRow>
    <div className="output-actions">
      <button className="settings-button settings-button--compact" disabled={disabled || busy} onClick={() => { setText(false); void load(true, false); }}>{c.fetchModels}</button>
      {hasTextCatalog && <button className="settings-button settings-button--compact" disabled={disabled || busy} onClick={() => { setText(true); void load(true, true); }}>{c.fetchModels} — {c.textModels}</button>}
    </div>
    {!!view?.models.length && <SettingsRow label={c.catalog}><SettingsSelect label={c.catalog} value={model} disabled={disabled || busy} options={view.models.map(m => ({ value: m.id, label: m.name, disabled: !m.compatible }))} onChange={setModel} /></SettingsRow>}
    {view?.editable && <SettingsRow label={c.manual}><span className="output-model-editor"><input className="output-text-input" aria-label={c.manual} dir="ltr" autoComplete="off" spellCheck={false} value={model} disabled={disabled || busy} onChange={e => setModel(e.target.value)} /><button className="settings-button settings-button--compact" disabled={disabled || busy || !model.trim() || model.trim() === view.current[text ? 1 : 0]} onClick={() => void save()}>{c.saveModel}</button></span></SettingsRow>}
    {busy && <InlineFeedback tone="info">{c.pending}</InlineFeedback>}
    {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
  </ServiceEditorModule>;
}
