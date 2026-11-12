// Text the settings page offers to copy so an agent host can connect to this program over stdio.
// The path is the real location of the running program; the host starts it with --mcp.

export type McpKind = "json" | "claude" | "codex" | "gemini";

const quote = (exe: string) => `"${exe.replaceAll('"', "")}"`;

export function mcpSetup(kind: McpKind, exe: string): string {
  if (kind === "claude")
    return `claude mcp add smallstocks-tw -- ${quote(exe)} --mcp`;
  if (kind === "codex")
    return `codex mcp add smallstocks-tw -- ${quote(exe)} --mcp`;
  const json = JSON.stringify(
    { mcpServers: { "smallstocks-tw": { command: exe, args: ["--mcp"] } } },
    null,
    2,
  );
  // Gemini CLI reads the same mcpServers shape from its own settings file; the line
  // above the JSON is an instruction for the human, not part of the file.
  if (kind === "gemini")
    return `Gemini CLI：把下面這段 JSON 貼到 ~/.gemini/settings.json（與既有設定合併；這一行為說明，不要貼）\n${json}`;
  return json;
}
