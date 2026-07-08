export function createMockWs() {
  const ls = new Set<(msg: string) => void>()
  return {
    send(d: object) { ls.forEach(f => f(JSON.stringify(d))) },
    on(f: (msg: string) => void) { ls.add(f); return () => ls.delete(f) },
    reset() { ls.clear() },
  }
}
