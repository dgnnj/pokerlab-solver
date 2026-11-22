// Live updater test against the PUBLIC feed (production key, GitHub Releases): installs a
// published release, checks that the launcher offers the current Latest, postpones once, then
// updates and verifies version, health banner and preserved history.
//
// Usage (setup.exe downloaded from the older release):
//   node tests/e2e/updater-live.mjs --setup <PokerLab_X_x64-setup.exe> --from 0.3.2 --to 0.3.3 \
//        [--data <dir>] [--shots out/updater-live-shots] [--timeout 600]
//
// Installs into %LOCALAPPDATA%\PokerLab (same place as a real install); user data goes to --data,
// by default under the system temp dir: with the data dir in a OneDrive-synced folder the run
// once hung with no output and, in another run, the app never created its database there.
// The whole run aborts after --timeout seconds.
import { chromium } from "playwright-core";
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const arg = (n, d) => {
  const i = process.argv.indexOf(`--${n}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const setup = path.resolve(arg("setup", ""));
const from = arg("from", "0.3.0");
const to = arg("to", "0.3.1");
const data = path.resolve(arg("data", path.join(os.tmpdir(), "pokerlab-updater-live-data")));
const shots = path.resolve(arg("shots", "out/updater-live-shots"));
const port = Number(arg("port", "9335"));
const exe = path.join(process.env.LOCALAPPDATA, "PokerLab", "pokerlab.exe");
const log = (...a) => console.log("[live]", ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const esc = (s) => s.replace(/\./g, "\\.");

const results = [];
const check = (name, cond, detail) => {
  results.push({ name, ok: !!cond });
  log(`${cond ? "OK  " : "FALHA"} ${name} — ${detail}`);
};
const fileVersion = () =>
  execFileSync("powershell", ["-NoProfile", "-Command", `(Get-Item '${exe}').VersionInfo.ProductVersion`]).toString().trim();
const killAll = () => {
  try {
    execFileSync("taskkill", ["/IM", "pokerlab.exe", "/T", "/F"], { stdio: "ignore" });
  } catch {
    /* none */
  }
};
const launch = () =>
  spawn(exe, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, POKERLAB_DATA_DIR: data },
    stdio: "ignore",
  });
async function connect(timeoutMs = 60000) {
  const t = Date.now();
  while (Date.now() - t < timeoutMs) {
    try {
      const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      for (let j = 0; j < 40; j++) {
        const page = browser.contexts().flatMap((c) => c.pages()).find((p) => !p.url().startsWith("devtools"));
        if (page) return { browser, page };
        await sleep(250);
      }
      await browser.close();
    } catch {
      /* retry */
    }
    await sleep(500);
  }
  throw new Error("DevTools não respondeu");
}
async function closeApp(child, browser) {
  await browser.close().catch(() => undefined);
  try {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T"], { stdio: "ignore" });
  } catch {
    /* gone */
  }
  for (let i = 0; i < 50 && child.exitCode === null; i++) await sleep(200);
  killAll();
  await sleep(500);
}
const sidebarVersion = async (page) => (await page.locator(".sidebar-foot").innerText()).trim();

const timeoutS = Number(arg("timeout", "600"));
setTimeout(() => {
  log(`FALHA tempo limite de ${timeoutS} s excedido; encerrando`);
  killAll();
  process.exit(1);
}, timeoutS * 1000).unref();

if (!existsSync(setup)) throw new Error(`--setup não encontrado: ${setup}`);
log(`dados em ${data}`);
rmSync(data, { recursive: true, force: true });
mkdirSync(data, { recursive: true });
mkdirSync(shots, { recursive: true });

// ---------- 0. install the published older release ----------
killAll();
execFileSync(setup, ["/S"], { timeout: 180000 }); // sync call: the run timer cannot fire meanwhile
check(`instalar ${from} publicada (NSIS silencioso)`, fileVersion() === from, `${exe} = ${fileVersion()}`);

// ---------- 1. offered from the public feed; postpone; create history ----------
{
  const child = launch();
  const { browser, page } = await connect();
  await page.getByRole("heading", { name: new RegExp(`Nova versão ${esc(to)}`) }).waitFor({ timeout: 30000 });
  const notes = await page.locator(".update-card .notes").innerText().catch(() => "");
  await page.screenshot({ path: path.join(shots, "01-oferta.png") });
  check(`feed público oferece ${to}`, true, notes.split("\n")[0]);
  await page.getByRole("button", { name: "Adiar" }).click();
  await page.getByText("Bem-vindo!").waitFor({ timeout: 20000 });
  await page.getByRole("button", { name: /Padrão — 20 decisões/ }).click();
  await page.getByRole("button", { name: "Treinar", exact: true }).first().click();
  await page.getByLabel("Número de decisões").fill("2");
  await page.getByRole("button", { name: "Começar" }).click();
  for (let i = 0; i < 2; i++) {
    await page.locator(".actions .action").first().click();
    await page.locator(".feedback").waitFor();
    await page.keyboard.press("Enter");
  }
  await page.getByRole("heading", { name: "Resumo da sessão" }).waitFor();
  const v = await sidebarVersion(page);
  check(`histórico criado na ${from}`, v.includes(`v${from}`), v);
  await closeApp(child, browser);
}

// ---------- 2. postponed version is skipped once ----------
{
  const child = launch();
  const { browser, page } = await connect();
  await page.getByText(new RegExp(`Versão ${esc(to)} adiada`)).waitFor({ timeout: 30000 });
  await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 20000 });
  check("adiar: pula uma abertura", true, `Versão ${to} adiada`);
  await closeApp(child, browser);
}

// ---------- 3. update through the public feed ----------
{
  const child = launch();
  let { browser, page } = await connect();
  await page.getByRole("heading", { name: new RegExp(`Nova versão ${esc(to)}`) }).waitFor({ timeout: 30000 });
  const t0 = Date.now();
  await page.getByRole("button", { name: "Atualizar agora" }).click();
  await page
    .getByText("A atualização falhou")
    .waitFor({ timeout: 4000 })
    .then(async () => check("download e assinatura", false, await page.locator(".alert-error").innerText()))
    .catch(() => undefined);
  await browser.close().catch(() => undefined);
  for (let i = 0; i < 400 && child.exitCode === null; i++) await sleep(300);
  check("app encerra para o instalador", child.exitCode !== null, `exit=${child.exitCode} após ${Math.round((Date.now() - t0) / 1000)} s`);
  let relaunched = null;
  let manual = false;
  try {
    relaunched = await connect(120000);
  } catch {
    log("o app relançado não expôs DevTools; abrindo manualmente para verificar");
    manual = true;
    killAll();
    launch();
    relaunched = await connect();
  }
  ({ browser, page } = relaunched);
  await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 30000 });
  const v = await sidebarVersion(page);
  check(`relança na ${to}`, v.includes(`v${to}`) && fileVersion() === to, `${v}, exe ${fileVersion()}, ${Math.round((Date.now() - t0) / 1000)} s; relançado pelo instalador: ${!manual}`);
  const banner = await page.locator(".alert-ok").innerText().catch(() => "");
  check("verificação de saúde após atualizar", new RegExp(`Atualizado de ${esc(from)} para ${esc(to)}`).test(banner), banner.replace(/\n/g, " "));
  await page.screenshot({ path: path.join(shots, "02-apos-atualizar.png") });
  await page.getByRole("button", { name: "Progresso", exact: true }).first().click();
  await page.locator("table.history tbody tr").first().waitFor();
  const n = await page.locator(".stat .stat-v").first().innerText();
  check("histórico preservado", Number(n) === 2, `n=${n}`);
  await browser.close();
  killAll();
}

const failed = results.filter((r) => !r.ok);
console.log(failed.length ? `[live] FALHOU (${failed.length})` : "[live] OK — todos os passos");
process.exit(failed.length ? 1 : 0);
