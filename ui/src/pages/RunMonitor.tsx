import { useState, useCallback, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { api } from '../lib/api'
import { useWebSocket } from '../hooks/useWebSocket'
import DAGViewer from '../components/DAGViewer'
import LogStream from '../components/LogStream'
import type { WsMessage, GraphData } from '../lib/types'

export default function RunMonitor() {
  const { id } = useParams<{ id: string }>()
  const [graph, setGraph] = useState<GraphData | null>(null)
  const [logs, setLogs] = useState<{ nodeId: string; text: string; ts: number }[]>([])
  const [status, setStatus] = useState('pending')

  const onMessage = useCallback((msg: WsMessage) => {
    switch (msg.type) {
      case 'node_status':
        setGraph(prev => prev ? {
          ...prev,
          nodes: prev.nodes.map(n =>
            n.id === msg.data.node_id ? { ...n, status: msg.data.status } : n
          ),
        } : prev)
        break
      case 'node_chunk':
        setLogs(prev => [...prev.slice(-200), {
          nodeId: msg.data.node_id,
          text: msg.data.text,
          ts: msg.data.ts,
        }])
        break
      case 'workflow_done':
        setStatus(msg.data.status)
        break
      case 'snapshot':
        setGraph(prev => prev ? {
          ...prev,
          nodes: prev.nodes.map(n => ({
            ...n,
            status: (msg.data.nodes[n.id] as GraphData['nodes'][0]['status']) ?? n.status,
          })),
          running_count: msg.data.running_count,
          elapsed_secs: msg.data.elapsed_secs,
        } : prev)
        break
    }
  }, [])

  useWebSocket(id ?? null, onMessage)

  // load initial graph data
  const { data: initialGraph } = useQuery({
    queryKey: ['run-graph', id],
    queryFn: () => api.runs.graphStatus(id!),
    enabled: !!id,
  })
  useEffect(() => {
    if (initialGraph) setGraph(initialGraph)
  }, [initialGraph])

  return (
    <div>
      <div className="flex items-center gap-3 mb-4">
        <h2 className="text-xl font-bold">运行监控</h2>
        <span className={`text-xs px-2 py-0.5 rounded ${
          status === 'running' ? 'bg-blue-900 text-blue-300' :
          status === 'completed' ? 'bg-emerald-900 text-emerald-300' :
          status === 'failed' ? 'bg-red-900 text-red-300' : 'bg-slate-800 text-slate-400'
        }`}>{status}</span>
      </div>
      {!id ? (
        <p className="text-slate-400">未指定运行 ID</p>
      ) : (
        <div className="grid grid-cols-3 gap-4">
          <div className="col-span-2">
            <DAGViewer graph={graph} />
          </div>
          <div>
            <LogStream logs={logs} />
          </div>
        </div>
      )}
    </div>
  )
}
