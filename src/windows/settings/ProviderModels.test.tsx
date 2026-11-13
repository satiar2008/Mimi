// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { ProviderModels } from "./ProviderModels";
const native = vi.hoisted(() => ({ get: vi.fn(), save: vi.fn() }));
vi.mock("../../lib/outputIpc", () => ({ providerModels: native.get, providerModelSave: native.save }));
let root: Root, host: HTMLDivElement;
const profile = { id: "gemini", name: "Gemini", provider: "googleGeminiLive", credentialState: "present" } as const;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); setStoredUiLanguage("fa-IR");
  native.get.mockReset().mockResolvedValue({ current: ["gemini-3.5-live-translate-preview"], models: [], editable: true }); native.save.mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
it("shows the current model without network enumeration until requested", async () => {
  await act(async () => root.render(<ProviderModels profile={profile} disabled={false} />));
  expect(host.querySelector('bdi[dir="ltr"]')?.textContent).toContain("gemini-3.5-live-translate-preview");
  expect(native.get).toHaveBeenCalledExactlyOnceWith("gemini");
  await act(async () => { window.dispatchEvent(new CustomEvent("mimi-model-refresh", { detail: "gemini" })); });
  expect(native.get).toHaveBeenLastCalledWith("gemini", true, false);
});
it("ignores a late response after changing the profile", async () => {
  let resolve!: (value: unknown) => void;
  native.get.mockImplementationOnce(() => new Promise(r => { resolve = r; }));
  await act(async () => root.render(<ProviderModels key="old" profile={profile} disabled={false} />));
  native.get.mockResolvedValue({ current: ["new-model"], models: [], editable: false });
  await act(async () => root.render(<ProviderModels key="new" profile={{ ...profile, id: "other" }} disabled={false} />));
  await act(() => resolve({ current: ["stale-model"], models: [], editable: true }));
  expect(host.textContent).toContain("new-model"); expect(host.textContent).not.toContain("stale-model");
});

it("refreshes the independent text model after its own successful connection check", async () => {
  await act(async () => root.render(<ProviderModels profile={{ ...profile, textTranslation: "openAICompatible" }} disabled={false} />));
  await act(async () => { window.dispatchEvent(new CustomEvent("mimi-model-refresh", { detail: { profileId: "gemini", stage: "text" } })); });
  expect(native.get).toHaveBeenLastCalledWith("gemini", true, true);
});
