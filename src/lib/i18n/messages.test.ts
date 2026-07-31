import { describe, expect, it } from "vitest";
import { LOCALES, loadMessages } from "./messages";

describe("locale message loading", () => {
  it("loads every supported locale with the complete English fallback", async () => {
    for (const locale of LOCALES) {
      const messages = await loadMessages(locale);
      expect(messages["common.loading"]).toBeTruthy();
      expect(messages["nav.dashboard"]).toBeTruthy();
    }
  });
});
