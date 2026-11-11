// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { microphoneTranslationCopy } from "../../lib/microphoneTranslation";
import { MicrophoneTranslationSettings } from "./MicrophoneTranslationSettings";
import { SettingsToastRegion } from "./SettingsToast";

const native = vi.hoisted(() => ({ insert: vi.fn(), cancel: vi.fn() }));
vi.mock("../../lib/ipc", async original => ({ ...await original<typeof import("../../lib/ipc")>(),
  microphoneInsertTranslation: native.insert, microphoneCancelInsertion: native.cancel }));
const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
const save = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("fa-IR"); save.mockReset(); native.insert.mockReset(); native.cancel.mockReset().mockResolvedValue(undefined);
  useStore.setState({ ...initial, saveSettings: save, initializationStatus: "ready", settings: { ...initial.settings,
    sourceLanguage: "en", targetLanguage: "fa", microphoneInputAvailable: true, microphoneTextInsertionAvailable: true,
    microphoneTranslation: { enabled: true, sourceLanguage: "fa", targetLanguage: null } } }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
const render = () => act(async () => root.render(<><MicrophoneTranslationSettings /><SettingsToastRegion /></>));
const switchButton = () => host.querySelector<HTMLButtonElement>('[role="switch"]')!;

it("offers independent Persian microphone languages without enabling microphone capture", async () => {
  await render();
  expect(host.textContent).toContain("هم‌زبان با گفتار دریافتی");
  expect(host.querySelector('button[aria-label="زبان گفتار میکروفون"]')?.textContent).toContain("فارسی");
  await act(async () => switchButton().click());
  expect(save).toHaveBeenCalledExactlyOnceWith({ microphoneTranslation: { enabled: false, sourceLanguage: "fa", targetLanguage: null } });
  expect(useStore.getState().settings.audioInput).toBe("system");
});

it("locks edits during listening and keeps the acknowledged value after a rejected save", async () => {
  save.mockRejectedValue(new Error("private failure"));
  await render(); await act(async () => switchButton().click());
  expect(switchButton().getAttribute("aria-checked")).toBe("true");
  expect(host.textContent).toContain(microphoneTranslationCopy().saveFailed);
  expect(host.textContent).not.toContain("private failure");
  await act(() => useStore.setState({ session: { ...initial.session, isActive: true, status: { kind: "listening" } } }));
  expect(switchButton().disabled).toBe(true);
});

it("keeps insertion disabled without a microphone final", async () => {
  await render();
  const button = [...host.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === microphoneTranslationCopy().insertAction)!;
  expect(button.disabled).toBe(true); expect(native.insert).not.toHaveBeenCalled();
});

it("cancels pending insertion when navigating away", async () => {
  native.insert.mockImplementation(() => new Promise(() => {}));
  useStore.setState({ session: { ...initial.session, isActive: true, status: { kind: "listening" },
    subtitles: { ...initial.session.subtitles, tracks: [{ audioSource: "microphone", source: { text: "گفتار", isFinal: true },
      translation: { text: "Speech", isFinal: true }, displayPair: { source: "گفتار", translation: "Speech" },
      displayPairFinal: true, displayPairCreatedAt: 1, history: [], detectedLanguage: null, isTranslationPending: false, isTranslationTimedOut: false }] } } });
  await render();
  const button = [...host.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === microphoneTranslationCopy().insertAction)!;
  await act(() => button.click());
  expect(native.insert).toHaveBeenCalledOnce(); expect(host.textContent).toContain(microphoneTranslationCopy().pending);
  await act(() => root.render(<div />));
  expect(native.cancel).toHaveBeenCalledOnce();
});
