import { invoke } from "@tauri-apps/api/core";
export interface CatalogItem { id: string; name: string; compatible: boolean }
export interface ModelView { current: string[]; models: CatalogItem[]; editable: boolean }
export interface TranslationOutput {
  shortcut: string; shortcutAction: "insert" | "copy"; autoCopy: boolean;
  speakMicrophone: boolean; speakSystem: boolean;
  speechProvider: "elevenLabs" | "openAICompatible"; endpoint: string;
  model: string; voice: string; speed: number; stability: number; similarity: number; device: string;
}
export interface OutputSnapshot { settings: TranslationOutput; keyPresent: boolean }
export const outputGet = () => invoke<OutputSnapshot>("translation_output_get");
export const outputSave = (settings: TranslationOutput, apiKey?: string, deleteKey = false) => invoke<OutputSnapshot>("translation_output_save", { settings, apiKey: apiKey ?? null, deleteKey });
export const outputDevices = () => invoke<{ id: string; name: string }[]>("speech_output_devices");
export const outputCatalog = (voices: boolean) => invoke<CatalogItem[]>("speech_output_catalog", { voices });
export const outputPreview = () => invoke<void>("speech_output_preview");
export const outputCancel = () => invoke<void>("speech_output_cancel");
export const microphoneCopy = () => invoke<void>("microphone_copy_translation");
export const providerModels = (profileId: string, fetch = false, text = false) => invoke<ModelView>("provider_models_get", { profileId, fetch, text });
export const providerModelSave = (profileId: string, model: string, text = false) => invoke<void>("provider_model_save", { profileId, model, text });
