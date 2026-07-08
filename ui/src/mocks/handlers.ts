import { http, HttpResponse } from 'msw'

export const handlers = [
  http.get('http://localhost:48080/api/workflows', () =>
    HttpResponse.json([{ id: 'wf-1', name: '示例工作流', node_count: 3, updated_at: '2026-07-08T12:00:00Z' }])),
  http.get('http://localhost:48080/api/workflows/:id/graph', () =>
    HttpResponse.json({ nodes: [{ id: 'A', label: 'fetch' }, { id: 'B', label: 'validate' }], edges: [{ from: 'A', to: 'B', label: 'complete' }], dataflows: [] })),
  http.get('http://localhost:48080/api/runs/:id/graph', () =>
    HttpResponse.json({ nodes: [{ id: 'A', label: 'fetch', status: 'Completed' }, { id: 'B', label: 'validate', status: 'Running' }], edges: [], dataflows: [], running_count: 1, elapsed_secs: 12 })),
]
