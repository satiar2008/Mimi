// @vitest-environment jsdom
import { act } from "react";
import permissions from "../../../src-tauri/permissions/app.toml?raw";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { VirtualMicrophoneSetup } from "./VirtualMicrophoneSetup";
const native=vi.hoisted(()=>({state:vi.fn(),prepare:vi.fn(),install:vi.fn(),use:vi.fn()}));
vi.mock("../../lib/outputIpc",()=>({virtualMicrophoneState:native.state,virtualMicrophonePrepare:native.prepare,virtualMicrophoneInstall:native.install,virtualMicrophoneUse:native.use}));
let root:Root,host:HTMLDivElement;
const configured=vi.fn();const errors:unknown[]=[];
const ready={installed:true,outputDeviceId:"synthetic-cable",inputName:"CABLE Output"};
const run=async(action:()=>Promise<void>)=>{try{await action();}catch(error){errors.push(error);}};
beforeEach(()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);setStoredUiLanguage("fa-IR");
  native.state.mockReset().mockResolvedValue({installed:false,outputDeviceId:null,inputName:null});
  native.prepare.mockReset().mockResolvedValue(undefined);native.install.mockReset().mockResolvedValue(ready);native.use.mockReset().mockResolvedValue(ready);configured.mockReset().mockResolvedValue(undefined);errors.length=0;
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(()=>root.unmount());host.remove();setStoredUiLanguage("system");vi.unstubAllGlobals();});
const render=(disabled=false)=>act(async()=>root.render(<VirtualMicrophoneSetup disabled={disabled} run={run} onConfigured={configured}/>));
const button=(text:string)=>[...document.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent===text)!;
it("reads device status without downloading, installing or changing output",async()=>{
  await render();expect(native.prepare).not.toHaveBeenCalled();expect(native.install).not.toHaveBeenCalled();expect(native.use).not.toHaveBeenCalled();
});
it("requires a separate confirmation after package preparation and honors cancel",async()=>{
  await render();await act(async()=>button("آماده‌کردن خروجی مجازی").click());
  expect(native.prepare).toHaveBeenCalledOnce();expect(native.install).not.toHaveBeenCalled();
  expect(document.querySelector('[role="dialog"]')!.textContent).toContain("VB-Audio");
  await act(()=>button("انصراف").click());expect(document.querySelector('[role="dialog"]')).toBeNull();expect(native.install).not.toHaveBeenCalled();
});
it("connects the microphone translation output only after installed endpoints are observed",async()=>{
  await render();await act(async()=>button("آماده‌کردن خروجی مجازی").click());await act(async()=>button("نصب خروجی مجازی").click());
  expect(native.install).toHaveBeenCalledOnce();expect(native.use).toHaveBeenCalledOnce();expect(configured).toHaveBeenCalledOnce();expect(host.textContent).toContain("CABLE Output");
});
it("grants virtual-device setup only to the settings webview",()=>{
  for (const command of ["virtual_microphone_state","virtual_microphone_prepare","virtual_microphone_install","virtual_microphone_use"]) {
    const grants=permissions.split("[[permission]]").filter(entry=>entry.includes(`"${command}"`));
    expect(grants).toHaveLength(1);
    expect(grants[0]).toContain('identifier = "app-settings"');
  }
});
it("keeps setup disabled for unsaved settings or an active session",async()=>{
  await render(true);expect(button("آماده‌کردن خروجی مجازی").disabled).toBe(true);expect(button("بررسی دوباره").disabled).toBe(true);expect(native.prepare).not.toHaveBeenCalled();
});
it("does not start installation when package verification fails",async()=>{
  native.prepare.mockRejectedValue("virtual_microphone_integrity");await render();await act(()=>button("آماده‌کردن خروجی مجازی").click());
  expect(errors).toEqual(["virtual_microphone_integrity"]);expect(native.install).not.toHaveBeenCalled();expect(document.querySelector('[role="dialog"]')).toBeNull();
});
