import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

// Run only against the installed app in credential-free MIMI_UI_TEST mode.
// Arguments: loopback CDP URL, dedicated UI-test preferences directory,
// optional directory for screenshots and the synthetic evidence report.
const [root, fixtureDirectory, evidenceDirectory] = process.argv.slice(2);
assert.ok(root && fixtureDirectory, "Usage: node scripts/verify-native-persian.mjs <CDP URL> <UI-test directory> [evidence directory]");
const endpoint = new URL(root);
assert.ok(endpoint.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(endpoint.hostname), "CDP must use a local loopback endpoint");
if (evidenceDirectory) await mkdir(evidenceDirectory, { recursive: true });
const report = [];
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(action, label) {
  for (let attempt = 0; attempt < 150; attempt++) {
    try { const value = await action(); if (value) return value; } catch {}
    await pause(100);
  }
  throw new Error("Timed out: " + label);
}
async function connect(target) {
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((ok, fail) => { socket.onopen = ok; socket.onerror = fail; });
  const requests = new Map();
  let id = 0;
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    if (!requests.has(message.id)) return;
    const [ok, fail] = requests.get(message.id);
    requests.delete(message.id);
    if (message.error) fail(message.error); else ok(message.result);
  };
  const call = (method, params = {}) => new Promise((ok, fail) => {
    requests.set(++id, [ok, fail]);
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (fn, argument) => {
    const expression = typeof fn === "string" ? fn : `(${fn})(${JSON.stringify(argument)})`;
    const result = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception.description);
    return result.result.value;
  };
  return { socket, call, evaluate };
}
const connections = [];
async function pages() {
  const targets = await (await fetch(root + "/json/list")).json();
  const result = {};
  for (const target of targets.filter(item => item.type === "page")) {
    const client = await connect(target);
    const label = await client.evaluate("window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label");
    if (label) { result[label] = client; connections.push(client); }
    else client.socket.close();
  }
  return result;
}
async function menu() {
  return JSON.parse(await readFile(join(fixtureDirectory, "native-menu.json"), "utf8"));
}
try {
  const initial = await until(async () => {
    const result = await pages();
    return result.settings ? result : false;
  }, "native settings window");
  const settings = initial.settings;
  await until(() => settings.evaluate('!!document.querySelector(".settings-console")'), "settings content");
  assert.equal(await settings.evaluate(async () => window.__TAURI_INTERNALS__.invoke("app_is_ui_test")), true,
    "Refusing to start a session in an ordinary app; launch with MIMI_UI_TEST=1");
  const visible = await settings.evaluate(() => {
    const button = [...document.querySelectorAll("button")].find(node => node.textContent.trim() === "General");
    button?.click();
    return !!button;
  });
  assert.ok(visible, "English General navigation");
  await until(() => settings.evaluate(() => !!document.querySelector('[role="combobox"][aria-label="Interface Language"]')), "language selector");
  await settings.evaluate(() => document.querySelector('[role="combobox"][aria-label="Interface Language"]').click());
  await until(() => settings.evaluate(() => [...document.querySelectorAll('[role="option"]')].some(node=>node.textContent.trim()==="فارسی")), "Persian language option");
  await settings.evaluate(() => [...document.querySelectorAll('[role="option"]')].find(node=>node.textContent.trim()==="فارسی").click());
  await until(() => settings.evaluate('document.documentElement.lang==="fa-IR" && document.documentElement.dir==="rtl"'), "live Persian settings");
  await until(async () => (await menu()).settings === "تنظیمات…", "Persian native menu");
  const nativeMenu = await menu();
  assert.equal(nativeMenu.session, "شروع زیرنویس");
  assert.equal(nativeMenu.subtitleDisplay, "نمایش زیرنویس");
  assert.deepEqual(nativeMenu.displayModes, ["فقط ترجمه", "متن اصلی و ترجمه", "فقط متن اصلی"]);
  assert.equal(nativeMenu.quit, "خروج از Mimi");
  report.push({ nativeMenu });
  await settings.evaluate(() => [...document.querySelectorAll("button")].find(node=>node.textContent.trim()==="زیرنویس").click());
  await until(() => settings.evaluate(() => !!document.querySelector('[role="switch"][aria-label="زیرنویس زنده"]')), "start control");
  await settings.evaluate(() => document.querySelector('[role="switch"][aria-label="زیرنویس زنده"]').click());
  await until(async () => (await menu()).session === "توقف زیرنویس", "native active session label");
  const active = await until(async () => {
    const result = await pages();
    return result.overlay && result["overlay-control"] ? result : false;
  }, "all three native windows");
  for (const [label, page] of Object.entries(active)) {
    await until(() => page.evaluate('document.documentElement.dir==="rtl"'), label + " direction");
    report.push({ window: label, dimensions: await page.evaluate("({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,lang:document.documentElement.lang})") });
  }
  const overlay = active.overlay;
  await until(() => overlay.evaluate('document.body.textContent.includes("به‌روزرسانی OpenAI")'), "Persian mixed subtitle");
  const content = await overlay.evaluate(() => [...document.querySelectorAll('span[dir="auto"]')].map(node => ({
    text: node.textContent, direction: getComputedStyle(node).direction, bidi: getComputedStyle(node).unicodeBidi,
  })));
  assert.ok(content.some(lane=>lane.direction==="rtl" && lane.text.includes("GPT-4o")));
  report.push({ subtitle: content });
  const control = active["overlay-control"];
  await until(() => control.evaluate('!!document.querySelector(".overlay-control-island")'), "native control island");
  await control.evaluate(() => document.querySelector(".overlay-control-island").click());
  await until(() => control.evaluate('!!document.querySelector(".overlay-control-panel")'), "expanded native controls");
  report.push({ controls: await control.evaluate("document.body.innerText") });
  if (evidenceDirectory) for (const [label, page] of Object.entries(active)) {
    const image = await page.call("Page.captureScreenshot", { format: "png" });
    await writeFile(join(evidenceDirectory, `mimi-native-${label}.png`), Buffer.from(image.data, "base64"));
  }
  const preferences = JSON.parse(await readFile(join(fixtureDirectory, "preferences.json"), "utf8"));
  assert.equal(preferences.ui_language, "fa-IR");
  report.push({ persistedLanguage: preferences.ui_language });
  if (evidenceDirectory) await writeFile(join(evidenceDirectory, "mimi-native-report.json"), JSON.stringify(report, null, 2));
  console.log("Native Persian windows and menu checks passed", JSON.stringify(report));
} finally {
  connections.forEach(client => client.socket.close());
}
