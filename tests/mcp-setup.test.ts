import { describe, expect, it } from "vitest";
import { mcpSetup } from "../src/mcp-setup.ts";

const win =
  "C:\\Users\\Ann Lee\\AppData\\Local\\MaxiStocks\\MaxiStocks.exe";
const mac = "/Applications/MaxiStocks.app/Contents/MacOS/MaxiStocks";

describe("MCP setup text", () => {
  it("makes JSON that a host can parse, with the real path and --mcp", () => {
    for (const exe of [win, mac]) {
      const parsed = JSON.parse(mcpSetup("json", exe));
      expect(parsed.mcpServers["maxi-stocks"]).toEqual({
        command: exe,
        args: ["--mcp"],
      });
    }
  });
  it("quotes paths with spaces in the one-line commands", () => {
    expect(mcpSetup("claude", mac)).toBe(
      `claude mcp add maxi-stocks -- "${mac}" --mcp`,
    );
    expect(mcpSetup("codex", win)).toBe(
      `codex mcp add maxi-stocks -- "${win}" --mcp`,
    );
  });
  it("gives Gemini CLI the same JSON, plus the file to paste it into", () => {
    const text = mcpSetup("gemini", mac);
    expect(text).toContain("~/.gemini/settings.json");
    const parsed = JSON.parse(text.slice(text.indexOf("{")));
    expect(parsed.mcpServers["maxi-stocks"]).toEqual({
      command: mac,
      args: ["--mcp"],
    });
  });
  it("never puts a token or connection file in the text", () => {
    for (const kind of ["json", "claude", "codex", "gemini"] as const)
      expect(mcpSetup(kind, mac)).not.toMatch(/token|mcp-connection/i);
  });
});
