import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { http, HttpResponse } from 'msw'
import { setupServer } from 'msw/node'

const server = setupServer(
  http.get('http://localhost:48080/api/workflows', () =>
    HttpResponse.json([{ id: 'wf-1', name: 'test', node_count: 3, updated_at: '2026-01-01T00:00:00Z' }])),
  http.get('http://localhost:48080/api/workflows/wf-1/graph', () =>
    HttpResponse.json({ nodes: [{ id: 'A', label: 'a' }], edges: [], dataflows: [] })),
)

beforeAll(() => server.listen())
afterAll(() => server.close())

describe('API Contract', () => {
  it('workflows list', async () => {
    const res = await fetch('http://localhost:48080/api/workflows')
    const data = await res.json()
    expect(data[0]).toHaveProperty('id')
    expect(data[0]).toHaveProperty('name')
    expect(typeof data[0].node_count).toBe('number')
  })
  it('graph', async () => {
    const res = await fetch('http://localhost:48080/api/workflows/wf-1/graph')
    const data = await res.json()
    expect(data).toHaveProperty('nodes')
    expect(data).toHaveProperty('edges')
    expect(Array.isArray(data.nodes)).toBe(true)
  })
})
