export function isNewerSequence(
  candidate: string,
  current: string | undefined,
): boolean {
  if (!/^\d+$/.test(candidate)) return false;
  if (current === undefined) return true;
  if (!/^\d+$/.test(current)) return true;
  return BigInt(candidate) > BigInt(current);
}
