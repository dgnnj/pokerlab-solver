// End-to-end UI test of the INSTALLED app (real pack, real SQLite), driven through the WebView2
// DevTools protocol. Flow: first launch -> train 3 decisions -> summary -> close -> relaunch ->
// Progress shows the same 3 attempts.
//
// Usage:
//   node tests/e2e/ui-persistence.mjs --exe "%LOCALAPPDATA%\PokerLab\pokerlab.exe" --data <empty dir> --shots <dir>
import { chromium } from "playwright-core";
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import path from "node:path";

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : def;
};
const exe = arg("exe", path.join(process.env.LOCALAPPDATA, "PokerLab", "pokerlab.exe"));
const data = path.resolve(arg("data", "out/e2e-data"));
const shots = path.resolve(arg("shots", "out/e2e-shots"));
const port = Number(arg("port", "9333"));
const decisions = Number(arg("decisions", "3"));

rmSync(data, { recursive: true, force: true });
mkdirSync(data, { recursive: true });
mkdirSync(shots, { recursive: true });

const log = (...a) => console.log("[e2e]", ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function launch() {
  const t0 = Date.now();
  const child = spawn(exe, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, POKERLAB_DATA_DIR: data },
    stdio: "ignore",
  });
  return { child, t0 };
}

async function connect() {
  for (let i = 0; i < 60; i++) {
    try {
      const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      for (let j = 0; j < 40; j++) {
        const page = browser.contexts().flatMap((c) => c.pages()).find((p) => !p.url().startsWith("devtools"));
        if (page) return { browser, page };
        await sleep(250);
      }
    } catch {
      /* not up yet */
    }
    await sleep(500);
  }
  throw new Error("WebView2 DevTools endpoint did not come up");
}

function close(child) {
  try {
    // Without /F taskkill sends WM_CLOSE: a normal window close.
    execFileSync("taskkill", ["/PID", String(child.pid), "/T"], { stdio: "ignore" });
  } catch {
    /* already gone */
  }
}

async function waitExit(child, ms = 15000) {
  const t = Date.now();
  while (child.exitCode === null && Date.now() - t < ms) await sleep(200);
  if (child.exitCode === null) {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  }
}

const results = { decisions: [] };

// ---------- Run 1: first launch and training ----------
let { child, t0 } = launch();
let { browser, page } = await connect();
await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 30000 });
results.startup_ms = Date.now() - t0;
log(`launcher -> Início em ${results.startup_ms} ms (url ${page.url()})`);
await page.screenshot({ path: path.join(shots, "01-inicio-primeira-abertura.png") });
const welcome = await page.getByText("Bem-vindo!").count();
log(`tela de boas-vindas visível: ${welcome > 0}`);
if (welcome) await page.getByRole("button", { name: /Padrão — 20 decisões/ }).click();

await page.getByRole("button", { name: "Treinar", exact: true }).first().click();
await page.getByRole("heading", { name: "Treinar" }).waitFor();
await page.getByLabel("Número de decisões").fill(String(decisions));
await page.getByRole("button", { name: "Começar" }).click();

for (let i = 0; i < decisions; i++) {
  await page.locator(".actions .action").first().waitFor();
  const hero = await page.locator(".hero .card").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
  const board = await page.locator(".board .card").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
  const theme = await page.locator(".train-head h1").innerText();
  const labels = await page.locator(".actions .action").allInnerTexts();
  const hiddenBefore = (await page.locator(".action-res").count()) === 0;
  await page.keyboard.press("1");
  await page.locator(".feedback").waitFor();
  const grade = await page.locator(".feedback h2").innerText();
  const table = await page.locator(".ev-table tbody tr").allInnerTexts();
  if (i === 0) await page.screenshot({ path: path.join(shots, "02-feedback.png") });
  results.decisions.push({ theme, hero, board, chosen: labels[0].replace(/^1\s*/, "").split("\n")[0], grade, hiddenBefore, table });
  log(`decisão ${i + 1}: ${theme} | ${hero.join(", ")} | ${grade.replace(/\n/g, " ")}`);
  await page.keyboard.press("Enter");
}
await page.getByRole("heading", { name: "Resumo da sessão" }).waitFor();
const summaryDecisions = await page.locator(".stat .stat-v").first().innerText();
await page.screenshot({ path: path.join(shots, "03-resumo.png") });
log(`resumo: ${summaryDecisions} decisões`);
await browser.close();
close(child);
await waitExit(child);
log("app fechado");

// ---------- Run 2: reopen and check history ----------
({ child, t0 } = launch());
({ browser, page } = await connect());
await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 30000 });
results.restart_ms = Date.now() - t0;
const welcome2 = await page.getByText("Bem-vindo!").count();
await page.getByRole("button", { name: "Progresso", exact: true }).first().click();
await page.getByRole("heading", { name: "Progresso" }).waitFor();
await page.locator("table.history tbody tr").first().waitFor();
const n = await page.locator(".stat .stat-v").first().innerText();
const rows = await page.locator("table.history tbody tr").allInnerTexts();
await page.screenshot({ path: path.join(shots, "04-progresso-apos-reabrir.png") });
await browser.close();
close(child);
await waitExit(child);

const combosShown = results.decisions.map((d) => d.hero);
let ok = true;
const fail = (m) => {
  ok = false;
  console.error("[e2e] FALHA:", m);
};
if (Number(summaryDecisions) !== decisions) fail(`resumo mostra ${summaryDecisions}`);
if (Number(n) !== decisions) fail(`progresso mostra n=${n}`);
if (rows.length !== decisions) fail(`histórico com ${rows.length} linhas`);
if (welcome2 !== 0) fail("boas-vindas reapareceu após reabrir");
if (!results.decisions.every((d) => d.hiddenBefore)) fail("estratégia visível antes da decisão");
if (!results.decisions.every((d) => d.table.length >= 2)) fail("tabela de EV incompleta");
log(`reabertura: n=${n}, linhas=${rows.length}, startup2=${results.restart_ms} ms`);
log("linhas do histórico:\n  " + rows.map((r) => r.replace(/\t/g, " | ")).join("\n  "));
log(`mãos jogadas: ${JSON.stringify(combosShown)}`);
console.log(ok ? "[e2e] OK" : "[e2e] FALHOU");
process.exit(ok ? 0 : 1);
