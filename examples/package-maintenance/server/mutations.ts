// Application-owned writes. Replace this module when Semantic DB writes arrive.
import { PrismaClient } from "@prisma/client";
export const db = new PrismaClient();
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
  return db.package.update({ where: { name }, data: packageInput(body) });
}
export async function updateTriage(
  issueId: string,
  body: Record<string, unknown>,
) {
  if (!issueId || issueId.length > 256)
    throw new InputError("Invalid issue ID");
  const data = triageInput(body);
  return db.issueTriage.upsert({
    where: { issueId },
    create: { issueId, ...data },
    update: data,
  });
}
