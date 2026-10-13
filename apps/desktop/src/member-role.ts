/** Role maps use full device identities. Short labels must never select a grant. */
export function memberRole(roles: Record<string, string>, identity: string): string {
  return roles[identity] ?? "member";
}

/** Legacy profile cards may show a badge only when their label resolves unambiguously. */
export function identityForRoleLabel(members: Array<{ fingerprint: string; identity: string }>, label: string): string {
  const matches = members.filter((m) => m.fingerprint === label);
  return matches.length === 1 ? matches[0].identity : "";
}
