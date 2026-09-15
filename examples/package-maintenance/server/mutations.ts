// All runtime mutations use the Semantic DB write dispatcher.
import { semantic } from "./db";
export class MissingRecordError extends Error {}
async function write(sql: string, values: unknown[]) {
  try { return await semantic.query(sql, values); }
  catch (error: any) {
    // A transport timeout/disconnect cannot establish whether the server committed.
    if (!/^[0-9A-Z]{5}$/.test(error?.code ?? "")) error.code = "08007";
    throw error;
  }
}
export class InputError extends Error {}
function object(body: unknown): asserts body is Record<string, unknown> {
  if (!body || typeof body !== "object" || Array.isArray(body))
    throw new InputError("A JSON object is required");
}
function text(value: unknown, label: string, max: number): string {
  if (typeof value !== "string" || value.length > max)
    throw new InputError(`${label} must be text of at most ${max} characters`);
  return value;
}
export function packageInput(body: unknown) {
  object(body);
  const team = text(body.team, "Team", 120).trim();
  if (!team) throw new InputError("Team is required");
  return { team, notes: text(body.notes, "Notes", 4000) };
}
export function triageInput(body: unknown) {
  object(body);
  const priority = body.priority;
  if (typeof priority !== "number" || ![1, 2, 3].includes(priority))
    throw new InputError("Priority must be 1, 2, or 3");
  const assigneeId =
    body.assignee_id === null ? null : text(body.assignee_id, "Assignee", 120);
  return { priority, assigneeId, notes: text(body.notes, "Notes", 4000) };
}
export async function updatePackage(
  name: string,
  body: Record<string, unknown>,
) {
  const { team, notes } = packageInput(body);
  const result = await write(
    "UPDATE packages SET team = $2::text, notes = $3::text WHERE name = $1::text",
    [name, team, notes],
  );
  if (result.rowCount === 0) throw new MissingRecordError("Record not found");
  return result;
}
export async function updateTriage(
  issueId: string,
  body: Record<string, unknown>,
) {
  if (!issueId || issueId.length > 256)
    throw new InputError("Invalid issue ID");
  const data = triageInput(body);
  return write(
    `MERGE INTO issue_triage AS target
     USING (SELECT $1::text AS issue_id, $2::text AS assignee_id,
                   $3::int AS priority, $4::text AS notes) AS source
     ON target.issue_id = source.issue_id
     WHEN MATCHED THEN UPDATE SET assignee_id = source.assignee_id,
       priority = source.priority, notes = source.notes
     WHEN NOT MATCHED THEN INSERT (issue_id, assignee_id, priority, notes)
       VALUES (source.issue_id, source.assignee_id, source.priority, source.notes)`,
    [issueId, data.assigneeId, data.priority, data.notes],
  );
}
