import { PrismaClient } from "@prisma/client";
const db = new PrismaClient();
try {
  for (const data of [
    {
      name: "requests",
      repository: "psf/requests",
      team: "Platform",
      notes: "Keep the client dependable for downstream services.",
    },
    {
      name: "requests-toolbelt",
      repository: "psf/requests",
      team: "Platform",
      notes: "Shared repository mapping for the example.",
    },
    {
      name: "urllib3",
      repository: "urllib3/urllib3",
      team: "Infrastructure",
      notes: "",
    },
    {
      name: "httpx",
      repository: "encode/httpx",
      team: "Developer tools",
      notes: "",
    },
    {
      name: "no-downloads-demo",
      repository: "example/quiet",
      team: "Community",
      notes: "No observed download data in this window.",
    },
  ])
    await db.package.upsert({
      where: { name: data.name },
      create: data,
      update: {},
    });
  for (const data of [
    { id: "alex", name: "Alex Chen" },
    { id: "sam", name: "Sam Rivera" },
    { id: "morgan", name: "Morgan Lee" },
  ])
    await db.member.upsert({
      where: { id: data.id },
      create: data,
      update: {},
    });
  await db.issueTriage.upsert({
    where: { issueId: "I_requests_1" },
    create: {
      issueId: "I_requests_1",
      assigneeId: "alex",
      priority: 1,
      notes: "Reproduce with a proxy before the next release.",
    },
    update: {},
  });
} finally {
  await db.$disconnect();
}
