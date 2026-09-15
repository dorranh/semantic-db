import { test } from "node:test";
import assert from "node:assert/strict";
import { packageInput, triageInput } from "../server/mutations";
import { count } from "../src/api";
test("input contracts reject invalid writes and preserve text", () => {
  assert.throws(() => packageInput({ team: "", notes: "" }));
  assert.throws(() => packageInput(null));
  assert.throws(() => triageInput([]));
  assert.throws(() =>
    triageInput({ priority: 0, assignee_id: null, notes: "" }),
  );
  assert.deepEqual(
    triageInput({
      priority: 1,
      assignee_id: null,
      notes: "'; DELETE FROM x; --",
    }),
    { priority: 1, assigneeId: null, notes: "'; DELETE FROM x; --" },
  );
  assert.equal(
    count("9007199254740993").replace(/\D/g, ""),
    "9007199254740993",
  );
  assert.equal(count(null), "No observation");
});
