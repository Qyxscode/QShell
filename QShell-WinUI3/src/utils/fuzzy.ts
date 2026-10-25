/**
 * 子序列模糊匹配：返回匹配字符在 text 中的下标数组，不匹配返回 null。
 * 例如 fuzzyMatchIndices("apt update", "au") -> [0, 4]
 */
export function fuzzyMatchIndices(text: string, query: string): number[] | null {
  if (!query) return [];
  const indices: number[] = [];
  let cursor = 0;
  for (const ch of query) {
    const idx = text.indexOf(ch, cursor);
    if (idx === -1) return null;
    indices.push(idx);
    cursor = idx + 1;
  }
  return indices;
}
