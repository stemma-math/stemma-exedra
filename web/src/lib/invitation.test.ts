import { describe, expect, it } from "vitest";
import { invitationToken } from "./invitation";

describe("invitations", () => {
  it("takes the token from a link, or as it is", () => {
    expect(invitationToken(" https://stemma.example.org/invite/abc123?x=1 ")).toBe("abc123");
    expect(invitationToken("/invite/abc123")).toBe("abc123");
    expect(invitationToken("abc123")).toBe("abc123");
    expect(invitationToken("https://stemma.example.org/signup?token=abc123")).toBe("abc123");
  });
});
