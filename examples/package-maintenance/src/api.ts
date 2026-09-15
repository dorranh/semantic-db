export interface Package {
  package_name: string;
  repository: string;
  team: string;
  notes: string;
  downloads: string | null;
  open_issues: string;
  unassigned_issues: string;
}
export interface Issue {
  issue_id: string;
  repository: string;
  number: string;
  title: string;
  state: string;
  url: string;
  assignee_id: string | null;
  assignee_name: string | null;
  priority: number;
  triage_notes: string;
}
export interface Member {
  id: string;
  name: string;
}
export interface Detail {
  package: Package;
  issues: Issue[];
  trend: { download_date: string; downloads: string }[];
}
export async function api<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const response = await fetch(`/api${path}`, {
    ...options,
    headers: { "Content-Type": "application/json", ...options.headers },
  });
  const body = await response.json();
  if (!response.ok) throw new Error(body.error ?? "Request failed");
  return body;
}
export const count = (value: string | null) =>
  value === null ? "No observation" : BigInt(value).toLocaleString();
