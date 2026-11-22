// Types for Jcode code plugins. Reference from your main module with
//   /** @param {import("./jcode").Jcode} jcode */
// or `import type { Jcode } from "./jcode"` in TypeScript.

export type HookEvent =
  | "pre_tool_transform" | "pre_tool" | "post_tool" | "post_tool_feedback"
  | "turn_start" | "turn_end" | "session_start" | "session_end";

export interface HookContext {
  event: HookEvent;
  sessionId: string;
  cwd: string;
  /** jcode tool name, e.g. "bash", "edit", "read". Empty for non-tool events. */
  tool: string;
  /** Tool input JSON for tool events. */
  input: Record<string, any>;
  /** Every JCODE_HOOK_* field jcode sent (see jcode docs/HOOKS.md). */
  payload: Record<string, any>;
}

export type HookResult =
  | void
  | false                         // pre_tool: block
  | { block: string | true }       // pre_tool: block with a reason for the model
  | { input: Record<string, any> } // pre_tool_transform: replace the tool input
  | { feedback: string }           // post_tool_feedback: text appended to the result
  | string;                        // post_tool_feedback: same as { feedback }

export interface CommandContext { root: string; args: string; cwd: string }
/** A string is sent as the prompt; { message } is shown in the chat instead. */
export type CommandResult = string | { prompt?: string; message?: string; error?: string } | void;

export interface ToolOptions {
  description?: string;
  /** JSON Schema for the tool arguments. */
  parameters?: Record<string, any>;
}

export type Node = Record<string, any>;
export interface Ui {
  stack(children: (Node | false | null)[], o?: { direction?: "vertical" | "horizontal"; gap?: Space; padding?: Space }): Node;
  row(children: (Node | false | null)[], o?: { gap?: Space }): Node;
  card(children: (Node | false | null)[], title?: string): Node;
  scroll(children: (Node | false | null)[], maxHeight?: number): Node;
  text(text: string, o?: { style?: "body" | "title" | "heading" | "caption" | "mono"; tone?: Tone }): Node;
  heading(text: string): Node;
  caption(text: string): Node;
  markdown(text: string): Node;
  code(text: string, language?: string): Node;
  divider(): Node;
  spacer(): Node;
  button(label: string, action: string, args?: object, o?: { variant?: "primary" | "secondary" | "compact" | "danger"; disabled?: boolean }): Node;
  chip(label: string, action?: string, args?: object): Node;
  input(bind: string, o?: { placeholder?: string; multiline?: boolean; onSubmit?: string }): Node;
  toggle(label: string, bind: string, onChange?: string): Node;
  select(bind: string, options: (string | { value: string; label: string })[], onChange?: string): Node;
  list(items: (Node | false | null)[]): Node;
  item(title: string, o?: { subtitle?: string; meta?: string; badges?: string[]; action?: string; args?: object }): Node;
  keyValue(rows: Record<string, string | number>): Node;
  table(columns: string[], rows: (string | number)[][]): Node;
  progress(value?: number, label?: string): Node;
  empty(title: string, detail?: string): Node;
  error(message: string): Node;
}
export type Space = "none" | "xs" | "sm" | "md" | "lg" | "xl";
export type Tone = "default" | "dim" | "accent" | "success" | "warning" | "danger";

export interface PanelOptions<S = Record<string, any>> {
  title?: string;
  placement?: "panel" | "sidebar" | "overlay";
  corner?: "top_right" | "bottom_right" | "bottom_left";
  /** How users open it: a sidebar pill (default), at startup, a command palette entry, or a shortcut. */
  launcher?: "sidebar" | "startup" | "command" | { shortcut: string };
  /** Host services: "open_url", "clipboard", "start_chat", "send_prompt", "read_files". */
  capabilities?: string[];
  /** Initial state. Inputs, toggles and selects bind to its keys. */
  state?: S;
  render(state: S, ui: Ui): Node | Promise<Node>;
  /** Button/input actions. Return the next state, or nothing to keep it. */
  actions?: Record<string, (args: any, state: S, ctx: { ui: Ui; toast(text: string, tone?: Tone): void }) => S | void | Promise<S | void>>;
}

export interface Jcode {
  /** Plugin folder. */
  root: string;
  name: string;
  ui: Ui;
  command(name: string, handler: (args: string, ctx: CommandContext) => CommandResult | Promise<CommandResult>): void;
  command(name: string, options: { description?: string; argumentHint?: string }, handler: (args: string, ctx: CommandContext) => CommandResult | Promise<CommandResult>): void;
  tool(name: string, options: ToolOptions, handler: (args: any, ctx: { root: string }) => any): void;
  on(event: HookEvent, handler: (event: HookContext) => HookResult | Promise<HookResult>): void;
  panel<S = Record<string, any>>(id: string, options: PanelOptions<S>): void;
  /** Writes to stderr (shown in hook logs). */
  log(...args: any[]): void;
}
