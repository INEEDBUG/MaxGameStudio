import { describe, expect, it } from "vitest";
import { leaguePrivacyText, maskLeagueName } from "./leagueStreamerMode";

describe("League streamer mode helpers", () => {
  it("uses stable aliases without exposing the source name", () => {
    const first = maskLeagueName("SecretName", 2, true, "puuid-1");
    expect(first).toBe(maskLeagueName("SecretName", 2, true, "puuid-1"));
    expect(first).not.toContain("SecretName");
  });

  it("uses numbered generic names and masks sensitive values", () => {
    expect(maskLeagueName("SecretName", 0, false)).toBe("召唤师 1");
    expect(leaguePrivacyText("secret#tag", true)).toBe("●●●●●●");
    expect(leaguePrivacyText("visible", false)).toBe("visible");
  });
});
