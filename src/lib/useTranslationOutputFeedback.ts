import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";
import { microphoneTranslationError } from "./microphoneTranslation";
import { outputCopy, outputError } from "./outputCopy";
import { useSettingsToast } from "../windows/settings/useSettingsToast";

/** A categorical global-action result uses the existing transient toast slot. */
export function useTranslationOutputFeedback() {
  const { beginToast } = useSettingsToast();
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false; let unlisten: (() => void) | undefined;
    void listen<string>("translation-output-feedback", ({ payload }) => {
      if (disposed) return;
      const failure = payload !== "output_done";
      beginToast()(failure ? microphoneTranslationError(payload) ?? outputError(payload) : outputCopy().done, failure);
    }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, [beginToast]);
}
