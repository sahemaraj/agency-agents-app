import { describe, expect, it } from "vitest";
import { defaultMessages, LOCALES, loadMessages } from "./messages";

describe("locale message loading", () => {
  it("loads every supported locale with the complete English fallback", async () => {
    const agentKeys = Object.keys(defaultMessages).filter((key) => key.startsWith("agents."));
    expect(agentKeys.length).toBeGreaterThan(100);
    for (const locale of LOCALES) {
      const messages = await loadMessages(locale);
      for (const key of agentKeys) expect(messages[key as keyof typeof messages]).toBeTruthy();
      expect(messages["common.loading"]).toBeTruthy();
      expect(messages["nav.dashboard"]).toBeTruthy();
      expect(messages["agents.manageSources"]).toBe("Manage Agent sources");
      expect(messages["agents.createAgent"]).toBe("Create Agent");
      expect(messages["state.sourceUnavailable"]).toBe("Source unavailable");
      expect(messages["agents.applyPlan"]).toBe("Apply plan");
      expect(messages["agents.policyHelp.autoTrusted"]).toContain("verified publisher key");
      expect(messages["agents.approvalInbox"]).toBe("Agent approval inbox");
      expect(messages["agents.renderedPreview"]).toBe("Rendered preview");
      expect(messages["agents.portableLibrary"]).toBe("Portable library");
    }
  });
});
