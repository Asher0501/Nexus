import { useEffect, useRef, useCallback } from 'react'
import type { WsMessage } from '../lib/types'

type Handler = (msg: WsMessage) => void

export function useWebSocket(runId: string | null, onMessage: Handler) {
  const wsRef = useRef<WebSocket | null>(null)
  const handlerRef = useRef(onMessage)
  handlerRef.current = onMessage

  useEffect(() => {
    if (!runId) return
    const url = `ws://127.0.0.1:48080/ws/runs/${runId}`
    const ws = new WebSocket(url)
    wsRef.current = ws
    ws.onmessage = (e) => {
      try { handlerRef.current(JSON.parse(e.data)) }
      catch { console.warn('[WS] parse error:', e.data) }
    }
    ws.onclose = () => { wsRef.current = null }
    return () => ws.close()
  }, [runId])

  return { send: useCallback((d: object) => wsRef.current?.send(JSON.stringify(d)), []) }
}
