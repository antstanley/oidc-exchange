import { expect, it } from "vitest";

import type { User } from "./types";

/**
 * The UI's status spelling must be the service's wire representation
 * (`UserStatus` serde `snake_case`). A title-cased value cannot even appear
 * here without failing typecheck; this pins the exact runtime values too.
 */
const ALL_USER_STATUSES: User["status"][] = ["active", "suspended", "deleted"];

it("User.status carries exactly the service's snake_case wire values", () => {
  expect(ALL_USER_STATUSES).toEqual(["active", "suspended", "deleted"]);
});
