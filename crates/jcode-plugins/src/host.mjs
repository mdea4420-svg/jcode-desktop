// Jcode code-plugin host. Written to ~/.jcode/plugins/.runtime/host.mjs by
// `jcode-plugins sync` and run with Node:
//
//   node host.mjs describe <main>               registrations as JSON
//   node host.mjs command  <main> <name> <args> run a slash command
//   node host.mjs hook     <main> <event>       jcode hook contract
//   node host.mjs mcp      <main>               MCP stdio server for tools
//   node host.mjs applet   <main> <id>          Jcode Desktop applet provider
//
// A plugin's main module default-exports `function (jcode) { ... }`.
import { pathToFileURL } from "node:url";
import { readFileSync } from "node:fs";
import path from "node:path";
import readline from "node:readline";

const [mode, main, ...rest] = process.argv.slice(2);
const EVENTS = [
  "pre_tool_transform", "pre_tool", "post_tool", "post_tool_feedback",
  "turn_start", "turn_end", "session_start", "session_end",
];

function fail(message, code = 1) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

// --- view helpers (jcode.applet/1 nodes) -----------------------------------
const act = (action, args) =>
  typeof action === "string" ? { action, args: args ?? {} } : action;
export const ui = {
  stack: (children, o = {}) => ({ type: "stack", direction: o.direction ?? "vertical", gap: o.gap ?? "sm", padding: o.padding ?? "none", children: children.filter(Boolean) }),
  row: (children, o = {}) => ui.stack(children, { ...o, direction: "horizontal" }),
  card: (children, title) => ({ type: "card", ...(title ? { title } : {}), children: children.filter(Boolean) }),
  scroll: (children, maxHeight) => ({ type: "scroll", ...(maxHeight ? { max_height: maxHeight } : {}), children: children.filter(Boolean) }),
  text: (text, o = {}) => ({ type: "text", text: String(text), style: o.style ?? "body", tone: o.tone ?? "default" }),
  heading: (text) => ui.text(text, { style: "heading" }),
  caption: (text) => ui.text(text, { style: "caption", tone: "dim" }),
  markdown: (text) => ({ type: "markdown", text: String(text) }),
  code: (text, language) => ({ type: "code", text: String(text), ...(language ? { language } : {}) }),
  divider: () => ({ type: "divider" }),
  spacer: () => ({ type: "spacer" }),
  button: (label, action, args, o = {}) => ({ type: "button", label, variant: o.variant ?? "secondary", on_press: act(action, args), disabled: !!o.disabled }),
  chip: (label, action, args) => ({ type: "chip", label, ...(action ? { on_press: act(action, args) } : {}) }),
  input: (bind, o = {}) => ({ type: "input", bind, placeholder: o.placeholder ?? "", multiline: !!o.multiline, ...(o.onSubmit ? { on_submit: act(o.onSubmit) } : {}) }),
  toggle: (label, bind, onChange) => ({ type: "toggle", label, bind, ...(onChange ? { on_change: act(onChange) } : {}) }),
  select: (bind, options, onChange) => ({ type: "select", bind, options: options.map((o) => (typeof o === "string" ? { value: o, label: o } : o)), ...(onChange ? { on_change: act(onChange) } : {}) }),
  list: (items) => ({ type: "list", children: items.filter(Boolean) }),
  item: (title, o = {}) => ({ type: "list_item", title, ...(o.subtitle ? { subtitle: o.subtitle } : {}), ...(o.meta ? { meta: o.meta } : {}), badges: o.badges ?? [], ...(o.action ? { on_press: act(o.action, o.args) } : {}) }),
  keyValue: (rows) => ({ type: "key_value", rows: Object.entries(rows).map(([key, value]) => ({ key, value: String(value) })) }),
  table: (columns, rows) => ({ type: "table", columns, rows: rows.map((r) => r.map(String)) }),
  progress: (value, label) => ({ type: "progress", ...(value == null ? {} : { value }), ...(label ? { label } : {}) }),
  empty: (title, detail) => ({ type: "empty", title, ...(detail ? { detail } : {}) }),
  error: (message) => ({ type: "error", message: String(message) }),
};

// --- load the plugin ---------------------------------------------------------
const registry = { commands: new Map(), tools: new Map(), events: new Map(), panels: new Map() };
const root = main ? path.dirname(path.resolve(main)) : process.cwd();
let manifest = {};
try { manifest = JSON.parse(readFileSync(path.join(root, ".claude-plugin", "plugin.json"), "utf8")); } catch {}
try { manifest = { ...JSON.parse(readFileSync(path.join(root, "plugin.json"), "utf8")), ...manifest }; } catch {}

const jcode = {
  root,
  name: manifest.name ?? path.basename(root),
  ui,
  command(name, options, handler) {
    if (typeof options === "function") [options, handler] = [{}, options];
    registry.commands.set(String(name), { options: options ?? {}, handler });
  },
  tool(name, options, handler) {
    registry.tools.set(String(name), { options: options ?? {}, handler });
  },
  on(event, handler) {
    if (!EVENTS.includes(event)) throw new Error(`unknown event "${event}" (one of ${EVENTS.join(", ")})`);
    if (!registry.events.has(event)) registry.events.set(event, []);
    registry.events.get(event).push(handler);
  },
  panel(id, options) {
    registry.panels.set(String(id), options ?? {});
  },
  log: (...args) => process.stderr.write(`[${manifest.name ?? path.basename(root)}] ${args.join(" ")}\n`),
};

async function load() {
  if (!main) fail("usage: host.mjs <mode> <main> ...");
  const module = await import(pathToFileURL(path.resolve(main)).href);
  const activate = module.default ?? module.activate;
  if (typeof activate !== "function") fail(`${main} must export a default function (jcode) { ... }`);
  await activate(jcode);
}

const readStdin = async () => {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8");
};

// --- modes ------------------------------------------------------------------
async function describe() {
  const out = {
    commands: [...registry.commands].map(([name, { options }]) => ({
      name, description: options.description ?? null, argument_hint: options.argumentHint ?? options.argument_hint ?? null,
    })),
    tools: [...registry.tools.keys()],
    events: [...registry.events.keys()],
    panels: [...registry.panels].map(([id, options]) => ({ id, title: options.title ?? id })),
  };
  process.stdout.write(JSON.stringify(out));
}

async function command() {
  const [name, args = ""] = rest;
  const entry = registry.commands.get(name);
  if (!entry) fail(`no command "${name}"`);
  const result = await entry.handler(args, { root, args, cwd: process.cwd() });
  const reply = typeof result === "string" ? { prompt: result }
    : result && typeof result === "object" ? result : { message: "Done." };
  process.stdout.write(JSON.stringify(reply));
}

async function hook() {
  const [event] = rest;
  const handlers = registry.events.get(event) ?? [];
  const env = process.env;
  const raw = ["pre_tool", "pre_tool_transform", "post_tool_feedback"].includes(event) ? await readStdin() : "";
  let input = {};
  try { input = raw ? JSON.parse(raw) : JSON.parse(env.JCODE_HOOK_TOOL_INPUT ?? "{}"); } catch {}
  let payload = {};
  try { payload = JSON.parse(env.JCODE_HOOK_PAYLOAD ?? "{}"); } catch {}
  const base = {
    event, sessionId: env.JCODE_HOOK_SESSION_ID ?? "", cwd: env.JCODE_HOOK_CWD ?? process.cwd(),
    tool: env.JCODE_HOOK_TOOL_NAME ?? "", payload,
  };
  if (event === "pre_tool") {
    for (const handler of handlers) {
      const result = await handler({ ...base, input });
      const reason = result === false ? "blocked" : result?.block;
      if (reason) { process.stderr.write(String(reason === true ? "blocked" : reason)); process.exit(2); }
    }
  } else if (event === "pre_tool_transform") {
    let changed = false;
    for (const handler of handlers) {
      const result = await handler({ ...base, input });
      if (result?.input && typeof result.input === "object") { input = result.input; changed = true; }
    }
    if (changed) process.stdout.write(JSON.stringify(input));
  } else if (event === "post_tool_feedback") {
    const parts = [];
    for (const handler of handlers) {
      const result = await handler({ ...base, input });
      const text = typeof result === "string" ? result : result?.feedback;
      if (text) parts.push(String(text));
    }
    process.stdout.write(parts.join("\n"));
  } else {
    for (const handler of handlers) {
      try { await handler({ ...base, input }); } catch (error) { jcode.log(error?.stack ?? error); }
    }
  }
}

async function mcp() {
  const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
  const tools = [...registry.tools].map(([name, { options }]) => ({
    name, description: options.description ?? "",
    inputSchema: options.parameters ?? options.inputSchema ?? { type: "object", properties: {} },
  }));
  const lines = readline.createInterface({ input: process.stdin });
  for await (const line of lines) {
    if (!line.trim()) continue;
    let request;
    try { request = JSON.parse(line); } catch { continue; }
    const { id, method, params } = request;
    if (id === undefined) continue; // notifications
    try {
      if (method === "initialize") {
        send({ jsonrpc: "2.0", id, result: {
          protocolVersion: params?.protocolVersion ?? "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name: `${jcode.name}-tools`, version: manifest.version ?? "0.0.0" },
        } });
      } else if (method === "tools/list") {
        send({ jsonrpc: "2.0", id, result: { tools } });
      } else if (method === "tools/call") {
        const entry = registry.tools.get(params?.name);
        if (!entry) throw new Error(`unknown tool ${params?.name}`);
        try {
          const result = await entry.handler(params?.arguments ?? {}, { root });
          const text = typeof result === "string" ? result : JSON.stringify(result ?? null, null, 2);
          send({ jsonrpc: "2.0", id, result: { content: [{ type: "text", text }] } });
        } catch (error) {
          send({ jsonrpc: "2.0", id, result: { content: [{ type: "text", text: String(error?.message ?? error) }], isError: true } });
        }
      } else if (method === "ping") {
        send({ jsonrpc: "2.0", id, result: {} });
      } else {
        send({ jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${method}` } });
      }
    } catch (error) {
      send({ jsonrpc: "2.0", id, error: { code: -32000, message: String(error?.message ?? error) } });
    }
  }
}

async function applet() {
  const [appletId] = rest;
  const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
  const panels = [...registry.panels];
  const placementOf = (options) => {
    const kind = options.placement ?? "panel";
    return kind === "overlay" ? { kind, corner: options.corner ?? "bottom_right" } : kind === "panel" ? { kind: "panel" } : { kind };
  };
  const launchers = panels.map(([id, options]) => {
    const trigger = options.launcher ?? "sidebar";
    const base = typeof trigger === "object" && trigger.shortcut
      ? { trigger: "shortcut", keys: trigger.shortcut }
      : trigger === "startup" ? { trigger: "startup" }
      : trigger === "command" ? { trigger: "command", label: options.title ?? id }
      : { trigger: "sidebar" };
    return { ...base, placement: placementOf(options), singleton: true };
  });
  send({ type: "register", manifest: {
    schema: process.env.JCODE_APPLET_SCHEMA ?? "jcode.applet/1",
    id: appletId, title: jcode.name,
    ...(manifest.description ? { description: manifest.description } : {}),
    launchers,
    capabilities: [...new Set(panels.flatMap(([, options]) => options.capabilities ?? []))],
  } });
  const instances = new Map(); // instance -> { panel, state, revision, placement }
  const render = async (instance) => {
    const entry = instances.get(instance);
    const options = registry.panels.get(entry.panel);
    let view;
    try { view = await options.render(entry.state, ui); } catch (error) { view = ui.error(error?.message ?? error); }
    entry.revision += 1;
    send({ type: "mount", instance, placement: entry.placement, document: {
      revision: entry.revision, title: options.title ?? entry.panel, state: entry.state ?? {}, view,
    } });
  };
  const lines = readline.createInterface({ input: process.stdin });
  for await (const line of lines) {
    let message;
    try { message = JSON.parse(line); } catch { continue; }
    try {
      if (message.type === "launch") {
        const [panel, options] = panels[message.launcher] ?? [];
        if (!panel) continue;
        instances.set(message.instance, { panel, state: structuredClone(options.state ?? {}), revision: 0, placement: placementOf(options) });
        await render(message.instance);
      } else if (message.type === "action") {
        const entry = instances.get(message.instance);
        if (!entry) continue;
        entry.state = { ...entry.state, ...(message.state ?? {}) };
        const handler = registry.panels.get(entry.panel).actions?.[message.action?.action];
        if (handler) {
          send({ type: "busy", instance: message.instance, busy: true });
          const next = await handler(message.action.args ?? {}, entry.state, { ui, toast: (text, tone = "default") => send({ type: "toast", instance: message.instance, text: String(text), tone }) });
          if (next && typeof next === "object") entry.state = next;
          send({ type: "busy", instance: message.instance, busy: false });
        }
        await render(message.instance);
      } else if (message.type === "resync") {
        if (instances.has(message.instance)) await render(message.instance);
      } else if (message.type === "closed") {
        instances.delete(message.instance);
      }
    } catch (error) {
      jcode.log(error?.stack ?? error);
    }
  }
}

const modes = { describe, command, hook, mcp, applet };
if (!modes[mode]) fail(`unknown mode "${mode}"`, 2);
await load();
await modes[mode]();
