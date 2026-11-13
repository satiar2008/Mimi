// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { outputCopy } from "../../lib/outputCopy";
import { useStore } from "../../lib/store";
import { TranslationOutputSettings } from "./TranslationOutputSettings";
const native = vi.hoisted(() => ({ get: vi.fn(), save: vi.fn(), devices: vi.fn(), catalog: vi.fn(), preview: vi.fn(), cancel: vi.fn() }));
vi.mock("../../lib/outputIpc", () => ({ outputGet: native.get, outputSave: native.save, outputDevices: native.devices, outputCatalog: native.catalog, outputPreview: native.preview, outputCancel: native.cancel, microphoneCopy: vi.fn() }));
let root: Root, host: HTMLDivElement;
const initial = useStore.getState();
const settings = { shortcut: "F8", shortcutAction: "insert", autoCopy: false, speakMicrophone: false, speakSystem: false, speechProvider: "elevenLabs", endpoint: "https://api.openai.com/v1/audio/speech", model: "", voice: "", speed: 1, stability: 0.5, similarity: 0.75, device: "" };
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); setStoredUiLanguage("fa-IR");
  native.get.mockReset().mockResolvedValue({ settings, keyPresent: false }); native.save.mockReset(); native.devices.mockReset().mockResolvedValue([{ id: "cable", name: "Virtual Cable" }]); native.catalog.mockReset(); native.preview.mockReset(); native.cancel.mockReset().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
const render = () => act(async () => root.render(<TranslationOutputSettings />));
function button(text: string) { return [...host.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === text)!; }
it("loads output choices without enabling microphone capture, speech or clipboard", async () => {
  await render();
  expect(host.textContent).toContain("خروجی ترجمه");
  expect(host.querySelector<HTMLInputElement>('input[aria-label="میانبر سراسری ترجمهٔ میکروفون"]')!.value).toBe("F8");
  expect(host.querySelector('[aria-label="خواندن ترجمهٔ صدای دریافتی"]')!.getAttribute("aria-checked")).toBe("false");
  expect(host.querySelector('[aria-label="کپی هر ترجمهٔ نهایی میکروفون"]')!.getAttribute("aria-checked")).toBe("false");
  expect(native.save).not.toHaveBeenCalled(); expect(native.catalog).not.toHaveBeenCalled(); expect(native.preview).not.toHaveBeenCalled();
  expect(useStore.getState().settings.audioInput).toBe(initial.settings.audioInput);
});
it("persists automatic clipboard as an explicit choice, retaining other settings", async () => {
  await render(); native.save.mockImplementation(async value => ({ settings: value, keyPresent: false }));
  await act(() => host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="کپی هر ترجمهٔ نهایی میکروفون"]')!.click());
  expect(native.save).not.toHaveBeenCalled();
  await act(() => button(outputCopy().save).click());
  expect(native.save).toHaveBeenCalledWith({ ...settings, autoCopy: true }, undefined, false, true);
  expect(button(outputCopy().save).disabled).toBe(true);
});
it("changes the incoming voice without changing microphone configuration", async () => {
  await render();
  native.save.mockImplementation(async value => ({ settings: value, keyPresent: true, systemKeyPresent: true }));
  const input = host.querySelector<HTMLInputElement>('input[aria-label="مدل خواننده"]')!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "incoming-model");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(() => button(outputCopy().save).click());
  const value = native.save.mock.calls[0][0];
  expect(value.systemSpeech.model).toBe("incoming-model");
  expect(value.model).toBe(settings.model);
  expect(value.speakMicrophone).toBe(false);
  expect(value.systemSpeech).not.toHaveProperty("shortcut");
});
it("enables incoming reading independently and previews only that path", async () => {
  const ready = { ...settings, systemSpeech: { speechProvider: "elevenLabs", endpoint: settings.endpoint, model: "eleven_v3", voice: "voice-id", device: "speakers", speed: 1, stability: 0.5, similarity: 0.75 } };
  native.get.mockResolvedValue({ settings: ready, keyPresent: false, systemKeyPresent: true });
  native.save.mockImplementation(async value => ({ settings: value, keyPresent: false, systemKeyPresent: true }));
  await render();
  await act(() => host.querySelector<HTMLButtonElement>('[aria-label="خواندن ترجمهٔ صدای دریافتی"]')!.click());
  await act(() => button(outputCopy().save).click());
  expect(native.save.mock.calls[0][0].speakSystem).toBe(true);
  expect(native.save.mock.calls[0][0].speakMicrophone).toBe(false);
  await act(() => button(outputCopy().preview).click());
  expect(native.preview).toHaveBeenCalledWith(true);
});
it("allows repairing an invalid model through its catalog without saving it first", async () => {
  native.get.mockResolvedValue({ settings, keyPresent: true, systemKeyPresent: true });
  native.catalog.mockResolvedValue([{ id: "eleven_v3", name: "Eleven v3", compatible: true }]);
  await render();
  const input = host.querySelector<HTMLInputElement>('input[aria-label="مدل خواننده"]')!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "Invalid display name");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(button(outputCopy().fetchModels).disabled).toBe(false);
  expect(button(outputCopy().preview).disabled).toBe(true);
  await act(() => button(outputCopy().fetchModels).click());
  expect(native.catalog).toHaveBeenCalledWith(false, true);
  expect(native.save).not.toHaveBeenCalled();
});
it("keeps a rejected shortcut draft available for correction", async () => {
  await render(); native.save.mockRejectedValue("output_shortcut_conflict");
  await act(() => host.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
  await act(() => button(outputCopy().save).click());
  expect(host.textContent).toContain(outputCopy().shortcutConflict);
  expect(button(outputCopy().save).disabled).toBe(false);
});
it("does not fetch catalogs or preview a voice before an API key is saved", async () => {
  await render();
  expect(button(outputCopy().fetchModels).disabled).toBe(true);
  expect(button(outputCopy().fetchVoices).disabled).toBe(true);
  expect(button(outputCopy().preview).disabled).toBe(true);
  await act(() => button(outputCopy().cancel).click()); expect(native.cancel).toHaveBeenCalledOnce();
});
