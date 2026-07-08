import { describe, it, expect } from 'vitest'

function parseWsMsg(raw: string): Record<string, unknown> | null {
  try {
    const p = JSON.parse(raw)
    return p?.type && p?.data ? p : null
  } catch { return null }
}

describe('WebSocket Contract', () => {
  it('parses node_status', () => {
    const msg = parseWsMsg(JSON.stringify({ type: 'node_status', data: { node_id: 'fetch', status: 'Running', ts: 1 } }))
    expect(msg?.data).toHaveProperty('node_id')
    expect(msg?.data).toHaveProperty('status')
    expect(msg?.data).toHaveProperty('ts')
  })
  it('parses node_chunk', () => {
    const msg = parseWsMsg(JSON.stringify({ type: 'node_chunk', data: { node_id: 'r', text: 'hello', ts: 1 } }))
    expect(msg?.data).toHaveProperty('text')
  })
  it('parses snapshot', () => {
    const msg = parseWsMsg(JSON.stringify({ type: 'snapshot', data: { running_count: 1, elapsed_secs: 5, nodes: {} } }))
    expect(msg?.data).toHaveProperty('running_count')
    expect(msg?.data).toHaveProperty('nodes')
  })
  it('parses workflow_done', () => {
    const msg = parseWsMsg(JSON.stringify({ type: 'workflow_done', data: { status: 'completed', duration_secs: 10 } }))
    expect(msg?.data).toHaveProperty('status')
  })
  it('parses error', () => {
    const msg = parseWsMsg(JSON.stringify({ type: 'error', data: { message: 'err' } }))
    expect(msg?.data).toHaveProperty('message')
  })
  it('rejects malformed json', () => {
    expect(parseWsMsg('not json')).toBeNull()
  })
  it('rejects missing type', () => {
    expect(parseWsMsg(JSON.stringify({ data: {} }))).toBeNull()
  })
  it('handles unknown type', () => {
    expect(parseWsMsg(JSON.stringify({ type: 'unknown', data: {} }))).not.toBeNull()
  })
})
