const BASE = 'http://localhost:48080'

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${url}`, init)
  if (!res.ok) throw new Error(`API ${res.status}: ${res.statusText}`)
  return res.json()
}

export const api = {
  workflows: {
    list: () => fetchJson<import('./types').WorkflowSummary[]>('/api/workflows'),
    getById: (id: string) => fetchJson<Record<string, unknown>>(`/api/workflows/${id}`),
    graph: (id: string) => fetchJson<import('./types').GraphData>(`/api/workflows/${id}/graph`),
  },
  runs: {
    getById: (id: string) => fetchJson<Record<string, unknown>>(`/api/runs/${id}`),
    graphStatus: (id: string) => fetchJson<import('./types').GraphData>(`/api/runs/${id}/graph`),
    trigger: (wfId: string) =>
      fetchJson<{ run_id: string }>(`/api/workflows/${wfId}/run`, { method: 'POST' }),
  },
}
