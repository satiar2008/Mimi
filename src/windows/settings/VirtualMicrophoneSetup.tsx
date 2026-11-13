import { useEffect, useRef, useState } from "react";
import { effectiveUiLanguage } from "../../lib/i18n";
import { virtualMicrophoneState, virtualMicrophonePrepare, virtualMicrophoneInstall, virtualMicrophoneUse, type VirtualMicrophoneState } from "../../lib/outputIpc";
import { SettingsConfirmation } from "./DestructiveConfirmation";
import { InlineFeedback, SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

const copy = () => effectiveUiLanguage() === "fa-IR" ? {
  title: "خروجی مجازی برنامهٔ تماس", prepare: "آماده‌کردن خروجی مجازی", use: "استفاده از خروجی مجازی", refresh: "بررسی دوباره", confirm: "نصب خروجی مجازی", cancel: "انصراف",
  question: "خروجی مجازی را نصب می‌کنید؟", details: "این خروجی با درایور رسمی VB-CABLE از VB-Audio ساخته می‌شود. نصب به مجوز مدیر ویندوز و احتمالاً راه‌اندازی دوباره نیاز دارد. VB-CABLE نرم‌افزار Donationware است و امکان حمایت یا خرید مجوز از سازنده وجود دارد.",
  source: "صفحهٔ سازنده و شرایط استفاده", ready: "خروجی ترجمهٔ میکروفون به کابل مجازی متصل شد.", notSeen: "خروجی مجازی هنوز دیده نمی‌شود؛ اگر نصب را انجام داده‌اید، ویندوز را راه‌اندازی دوباره و سپس «بررسی دوباره» را بزنید.",
  input: "در برنامهٔ تماس، این میکروفون را انتخاب کنید:", help: "فقط صدای ترجمهٔ میکروفون به این خروجی فرستاده می‌شود. ورودی میکروفون و زبان‌های آن را جدا انتخاب کنید. برای دریافت گفتار خودتان، میکروفون فیزیکی ویندوز را پیش‌فرض نگه دارید.", failed: "وضعیت خروجی مجازی دریافت نشد؛ دوباره بررسی کنید.",
} : {
  title: "Virtual call output", prepare: "Prepare virtual output", use: "Use virtual output", refresh: "Check again", confirm: "Install virtual output", cancel: "Cancel",
  question: "Install the virtual output?", details: "This output uses the official VB-CABLE driver by VB-Audio. Installation needs Windows administrator approval and may require a restart. VB-CABLE is donationware; you can support or license it from its publisher.",
  source: "Publisher and terms", ready: "Microphone translation output is connected to the virtual cable.", notSeen: "The virtual output is not visible yet. If installation completed, restart Windows and check again.", input: "In your call app, select this microphone:",
  help: "Only translated microphone speech is sent here. Choose microphone input and languages separately. Keep your physical microphone as the Windows default when capturing your own voice.", failed: "Could not read virtual-device status. Check again.",
};

export function VirtualMicrophoneSetup({ disabled, run, onConfigured }: {
  disabled: boolean; run: (action: () => Promise<void>) => Promise<void>; onConfigured: () => Promise<void>;
}) {
  const [state, setState] = useState<VirtualMicrophoneState | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [failed, setFailed] = useState(false);
  const mounted = useRef(false); const { beginToast } = useSettingsToast(); const c = copy();
  useEffect(() => {
    mounted.current = true;
    void virtualMicrophoneState().then(value => { if (mounted.current) setState(value); }).catch(() => { if (mounted.current) setFailed(true); });
    return () => { mounted.current = false; };
  }, []);
  const configureOutput = async () => {
    const next = await virtualMicrophoneUse();
    await onConfigured();
    if (mounted.current) { setState(next); beginToast()(c.ready); }
  };
  return <>
    <SettingsRow label={c.title} description={c.help} feedback={state?.installed ? <span>{c.input} <bdi>{state.inputName}</bdi></span> : failed ? <InlineFeedback tone="error">{c.failed}</InlineFeedback> : undefined}>
      <div className="output-actions virtual-microphone-actions">
        <button className="settings-button settings-button--compact" disabled={disabled || !state} onClick={() => void run(async () => {
          if (state?.installed) await configureOutput();
          else { await virtualMicrophonePrepare(); if (mounted.current) setConfirm(true); }
        })}>{state?.installed ? c.use : c.prepare}</button>
        <button className="settings-button settings-button--compact" disabled={disabled} onClick={() => void run(async () => { const next=await virtualMicrophoneState(); if (mounted.current) { setState(next); setFailed(false); } })}>{c.refresh}</button>
      </div>
    </SettingsRow>
    {confirm && <SettingsConfirmation variant="default" confirmIcon="checkmark" message={c.question} confirmLabel={c.confirm} cancelLabel={c.cancel} onCancel={() => setConfirm(false)} onConfirm={() => void run(async () => {
      setConfirm(false); const next=await virtualMicrophoneInstall();
      if (mounted.current) setState(next);
      if (next.installed) await configureOutput(); else if (mounted.current) beginToast()(c.notSeen);
    })}>
      <p>{c.details}</p><a href="https://vb-audio.com/Services/licensing.htm" target="_blank" rel="noreferrer">{c.source}</a>
    </SettingsConfirmation>}
  </>;
}
