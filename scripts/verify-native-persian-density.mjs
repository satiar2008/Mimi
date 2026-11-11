import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
// This verifies emulated WebView raster density, not Windows monitor DPI.
// Launch the installed app with MIMI_UI_TEST=1, Persian UI, and MIMI_AUTO_START=1.
const endpoint=process.argv[2];
const evidence=process.argv[3];
assert.ok(endpoint && evidence, "Usage: node scripts/verify-native-persian-density.mjs <loopback CDP URL> <evidence directory>");
const endpointUrl=new URL(endpoint);
assert.ok(endpointUrl.protocol==='http:' && ['127.0.0.1','localhost','[::1]'].includes(endpointUrl.hostname), 'CDP must use loopback');
const clients = [];
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn) {
  for (let i = 0; i < 100; i++) {
    try { const value = await fn(); if (value) return value; } catch {}
    await delay(100);
  }
  throw new Error("Native UI wait timed out");
}
async function connect(target) {
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise(resolve => socket.onopen = resolve);
  const pending = new Map();
  let id = 0;
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    if (pending.has(message.id)) {
      const [ok, fail] = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) fail(message.error); else ok(message.result);
    }
  };
  const call = (method, params) => new Promise((ok, fail) => {
    pending.set(++id, [ok, fail]);
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (fn, arg) => {
    const result = await call("Runtime.evaluate", {
      expression: `(${fn})(${JSON.stringify(arg)})`, awaitPromise: true, returnByValue: true,
    });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception.description);
    return result.result.value;
  };
  clients.push({ socket, call, evaluate });
  return clients.at(-1);
}
const windows = {};
for (const target of await (await fetch(new URL("/json/list", endpointUrl))).json()) {
  const page = await connect(target);
  const label = await page.evaluate(() => window.__TAURI_INTERNALS__.metadata.currentWindow.label);
  windows[label] = page;
}
const settings = windows.settings;
const reports = [];
assert.ok(settings, "Expected the installed native settings window");
await mkdir(evidence, { recursive: true });
try {
  assert.equal(await settings.evaluate(async () => window.__TAURI_INTERNALS__.invoke("app_is_ui_test")), true);
  await until(() => settings.evaluate(() => document.documentElement.lang === "fa-IR"));
  for (const density of [1.25, 1.5]) {
    for (const width of [760, 520]) {
      await settings.call("Emulation.setDeviceMetricsOverride", { width, height: 720, deviceScaleFactor: density, mobile: false });
      for (const theme of ["روشن", "تیره"]) {
        await settings.evaluate(() => [...document.querySelectorAll(".settings-sidebar button")].find(n=>n.textContent.trim()==="عمومی").click());
        await until(() => settings.evaluate(() => !!document.querySelector(".appearance-option")));
        await settings.evaluate(theme => [...document.querySelectorAll(".appearance-option")].find(n=>n.textContent.trim()===theme).click(), theme);
        await until(() => settings.evaluate(theme => document.querySelector(".settings-console").classList.contains(theme==="تیره"?"settings-console--dark":"settings-console--light"), theme));
        for (const page of ["زیرنویس", "صدا و زبان‌ها", "مدل‌ها و سرویس‌ها", "ذخیره و خروجی", "عمومی", "راهنما"]) {
          await settings.evaluate(page => [...document.querySelectorAll(".settings-sidebar button")].find(n=>n.textContent.trim()===page).click(), page);
          await delay(120);
          const result = await settings.evaluate(() => {
            const sidebar=document.querySelector('.settings-sidebar').getBoundingClientRect();
            const scroller=document.querySelector('.settings-console__scroll');
            return {lang:document.documentElement.lang,dir:document.documentElement.dir,dpr:devicePixelRatio,width:innerWidth,
              rootOverflow:document.documentElement.scrollWidth-innerWidth,
              contentOverflow:scroller.scrollWidth-scroller.clientWidth,
              sidebarOnRight:sidebar.left>=scroller.getBoundingClientRect().right-1};
          });
          assert.equal(result.dir,"rtl");
          assert.equal(result.lang,"fa-IR");
          assert.equal(result.width,width);
          assert.equal(result.dpr,density);
          assert.ok(result.rootOverflow<=1 && result.contentOverflow<=1,JSON.stringify({density,width,theme,page,result}));
          assert.equal(result.sidebarOnRight,true);
          reports.push({surface:"settings",density,width,theme,page,...result});
          if (page==="زیرنویس" || page==="عمومی") {
            const png=await settings.call("Page.captureScreenshot",{format:"png"});
            const suffix=theme==="تیره"?"dark":"light";
            await writeFile(`${evidence}/settings-${density}-${width}-${suffix}-${page==="زیرنویس"?"subtitle":"general"}.png`,Buffer.from(png.data,"base64"));
          }
        }
      }
    }
    for (const label of ["overlay", "overlay-control"]) {
      const client=windows[label];
      assert.ok(client,label);
      const width=label==="overlay"?760:280;
      const height=label==="overlay"?340:560;
      await client.call("Emulation.setDeviceMetricsOverride",{width,height,deviceScaleFactor:density,mobile:false});
      if(label==="overlay-control") {
        const mode=await client.evaluate(async()=>window.__TAURI_INTERNALS__.invoke('overlay_control_state'));
        if(mode!=='panel') await client.evaluate(()=>document.querySelector('.overlay-control-island').click());
        await until(()=>client.evaluate(async()=>
          await window.__TAURI_INTERNALS__.invoke('overlay_control_state')==='panel' &&
          document.querySelector('.overlay-control-panel')?.getBoundingClientRect().height>100));
        await client.evaluate(async()=>{await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame);});
      }
      const metrics=await client.evaluate(()=>({dir:document.documentElement.dir,lang:document.documentElement.lang,dpr:devicePixelRatio,
        lanes:[...document.querySelectorAll('span[dir="auto"]')].map(n=>({text:n.textContent,dir:getComputedStyle(n).direction,bidi:getComputedStyle(n).unicodeBidi})),
        overflow:document.documentElement.scrollWidth-innerWidth}));
      assert.equal(metrics.dir,"rtl");
      assert.equal(metrics.dpr,density);
      assert.ok(metrics.overflow<=1);
      if(label==="overlay") {
        assert.ok(metrics.lanes.some(n=>n.text.includes('GPT-4o')&&n.dir==='rtl'&&n.bidi==='plaintext'));
        assert.ok(metrics.lanes.some(n=>n.text.startsWith('Mimi')&&n.dir==='ltr'));
      }
      reports.push({surface:label,density,...metrics});
      if(label==='overlay-control') assert.equal(await client.evaluate(async()=>window.__TAURI_INTERNALS__.invoke('overlay_control_state')),'panel');
      const png=await client.call("Page.captureScreenshot",{format:"png"});
      await writeFile(`${evidence}/${label}-${density}.png`,Buffer.from(png.data,"base64"));
    }
  }
  await writeFile(join(evidence, "report.json"),JSON.stringify({method:"CDP emulated raster density in installed WebView2; Windows display scaling unchanged",reports},null,2));
  console.log(`Passed ${reports.length} installed-WebView checks at emulated 125%/150% raster density. Windows display DPI was not changed.`);
} finally {
  for(const page of Object.values(windows)) {
    try {await page.call("Emulation.clearDeviceMetricsOverride",{});} catch {}
  }
  clients.forEach(client=>client.socket.close());
}

