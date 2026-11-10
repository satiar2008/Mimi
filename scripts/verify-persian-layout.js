// Uses the real desktop frontend and synthetic native responses in the settings
// fixture. Run against a browser or Windows WebView2 supporting the Page adapter.
export async function verifyPersianLayout(page, baseUrl = "http://127.0.0.1:1420") {
  const failures = [];
  const require = (result, label) => { if (!result) failures.push(label); };
  const setup = async width => {
    await page.cdp("Emulation.setDeviceMetricsOverride", {
      width, height: 760, deviceScaleFactor: 1, mobile: false,
    });
    await page.goto(new URL("/scripts/fixtures/settings-layout.html?language=fa-IR", baseUrl).href);
    await page.waitForFunction(() => window.layoutReady === true && document.documentElement.lang === "fa-IR");
  };
  try {
    for (const windowLabel of ["settings", "overlay", "overlay-control"]) {
      await page.goto(new URL(`/?window=${windowLabel}`, baseUrl).href);
      await page.waitForFunction(() => document.readyState === "complete" && !!document.querySelector("#root"));
      const direction = await page.evaluate(async () => {
        const { setStoredUiLanguage } = await import("/src/lib/i18n.ts");
        const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
        await frame();
        await frame();
        setStoredUiLanguage("fa-IR");
        for (let i = 0; i < 60 && document.documentElement.dir !== "rtl"; i++) await frame();
        const persian = document.documentElement.lang === "fa-IR" && document.documentElement.dir === "rtl";
        setStoredUiLanguage("en");
        await frame();
        await frame();
        return persian && document.documentElement.dir === "ltr";
      });
      require(direction, `${windowLabel} in-place RTL and LTR`);
    }
    for (const width of [520, 952]) {
      await setup(width);
      for (const theme of ["light", "dark"]) {
        for (const category of ["voice-settings", "service-profiles", "subtitle-settings", "application-settings",
          "session-export", "diagnostics", "getting-started"]) {
          const result = await page.evaluate(async ({ category, theme, width }) => {
            await window.workspaceFixture.apply({ category, theme, language: "fa-IR", platform: "windows",
              listProfiles: category === "service-profiles", sessions: "library", subtitleAppearance: { subtitleDisplayMode: "bilingual" } });
            const sidebar = document.querySelector(".settings-sidebar");
            const scroll = document.querySelector(".settings-console__scroll");
            const preview = [...document.querySelectorAll(".subtitle-preview__text > *")];
            return {
              direction: document.documentElement.dir === "rtl" && sidebar.getBoundingClientRect().right > width / 2,
              overflow: scroll.scrollWidth > scroll.clientWidth + 1,
              preview: preview.length === 2 && preview.map(el => getComputedStyle(el).direction).join() === "ltr,rtl",
            };
          }, { category, theme, width });
          require(result.direction && !result.overflow, `${width}/${theme}/${category} layout`);
          if (category === "subtitle-settings") require(result.preview, `${width}/${theme} bilingual preview`);
        }
      }
      const picker = await page.evaluate(async () => {
        await window.workspaceFixture.apply({ category: "application-settings", language: "en" });
        const trigger = document.querySelector('[role="combobox"][aria-label="Interface Language"]');
        trigger.click();
        await new Promise(resolve => requestAnimationFrame(resolve));
        const persian = [...document.querySelectorAll('[role="option"]')].find(option => option.textContent.trim() === "فارسی");
        persian?.click();
        for (let frame = 0; frame < 4; frame++) await new Promise(resolve => requestAnimationFrame(resolve));
        return !!persian && localStorage.getItem("mimi-ui-language") === "fa-IR" &&
          [...document.querySelectorAll('[role="combobox"]')].some(element => element.textContent.includes("فارسی"));
      });
      require(picker, `${width} Persian selection and persistence`);
      const fields = await page.evaluate(async () => {
        await window.workspaceFixture.apply({ category: "service-profiles", language: "fa-IR", platform: "windows",
          editor: true, provider: "customOpenAIASR", textTranslation: "openAICompatible" });
        const inputs = [...document.querySelectorAll(".config-input-group input")];
        const technical = inputs.filter(input => input.dir !== "auto");
        const names = inputs.filter(input => input.dir === "auto");
        return technical.length > 0 && names.length > 0 &&
          technical.every(input => getComputedStyle(input).direction === "ltr");
      });
      require(fields, `${width} independent technical and name input directions`);
      const controls = await page.evaluate(async () => {
        await window.workspaceFixture.apply({ surface: "overlay", language: "fa-IR", category: undefined,
          sourceLanguage: "en", targetLanguage: "fa" });
        const panel = document.querySelector(".overlay-control-panel");
        return getComputedStyle(panel).direction === "rtl" && panel.textContent.includes("انگلیسی") &&
          panel.textContent.includes("فارسی") && panel.scrollWidth <= panel.clientWidth + 1;
      });
      require(controls, `${width} localized overlay controls`);
    }
  } finally {
    await page.cdp("Emulation.clearDeviceMetricsOverride", {});
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return { windowCases: 3, settingsCases: 28, pickerCases: 2, fieldCases: 2, panelCases: 2 };
}
