import { invoke } from "@tauri-apps/api/core";
export interface CatalogItem { id: string; name: string; compatible: boolean }
export interface ModelView { current: string[]; models: CatalogItem[]; editable: boolean }
export interface SpeechSettings {
  speechProvider: "elevenLabs" | "openAICompatible" | "mainService"; endpoint: string;
  model: string; voice: string; speed: number; stability: number; similarity: number; device: string;
}
export interface TranslationOutput extends SpeechSettings {
  shortcut: string; shortcutAction: "insert" | "copy"; autoCopy: boolean;
  speakMicrophone: boolean; speakSystem: boolean;
  systemSpeech?: SpeechSettings; duckSystemAudio?: boolean; originalVolume?: number;
}
export interface OutputSnapshot { settings: TranslationOutput; keyPresent: boolean; systemKeyPresent?: boolean; mainService?: string | null }
export const outputGet = () => invoke<OutputSnapshot>("translation_output_get");
export const outputSave = (settings: TranslationOutput, apiKey?: string, deleteKey = false, incoming = false) => invoke<OutputSnapshot>("translation_output_save", { settings, apiKey: apiKey ?? null, deleteKey, incoming });
export const outputDevices = () => invoke<{ id: string; name: string }[]>("speech_output_devices");
export const outputCatalog = (voices: boolean, incoming = false) => invoke<CatalogItem[]>("speech_output_catalog", { voices, incoming });
export const outputPreview = (incoming = false) => invoke<void>("speech_output_preview", { incoming });
export const outputCancel = () => invoke<void>("speech_output_cancel");
export const microphoneCopy = () => invoke<void>("microphone_copy_translation");
export interface VirtualMicrophoneState { installed: boolean; outputDeviceId: string | null; inputName: string | null }
export const virtualMicrophoneState = () => invoke<VirtualMicrophoneState>("virtual_microphone_state");
export const virtualMicrophonePrepare = () => invoke<void>("virtual_microphone_prepare");
export const virtualMicrophoneInstall = () => invoke<VirtualMicrophoneState>("virtual_microphone_install");
export const virtualMicrophoneUse = () => invoke<VirtualMicrophoneState>("virtual_microphone_use");
export const providerModels = (profileId: string, fetch = false, text = false) => invoke<ModelView>("provider_models_get", { profileId, fetch, text });
export const providerModelSave = (profileId: string, model: string, text = false) => invoke<void>("provider_model_save", { profileId, model, text });
