import { useParams, Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { api } from '../lib/api'
import DAGViewer from '../components/DAGViewer'
import type { GraphData } from '../lib/types'

interface RunRecord {
  id: string
  status: string
  created_at: string
}

export default function WorkflowDetail() {
  const { id } = useParams<{ id: string }>()

  const { data: wf, isLoading: wfLoading } = useQuery({
    queryKey: ['workflow', id],
    queryFn: () => api.workflows.getById(id!),
    enabled: !!id,
  })

  const { data: graph } = useQuery({
    queryKey: ['workflow-graph', id],
    queryFn: () => api.workflows.graph(id!),
    enabled: !!id,
  })

  const { data: runs } = useQuery({
    queryKey: ['workflow-runs', id],
    queryFn: () => fetch(`/api/workflows/${id}/runs`).then<RunRecord[]>(r => r.json()).catch(() => []),
    enabled: !!id,
  })

  if (wfLoading) return <p className="text-slate-400">加载中...</p>
  if (!wf) return <p className="text-slate-400">工作流不存在</p>

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <div>
          <h2 className="text-xl font-bold">{(wf as Record<string, unknown>).name as string ?? '未命名'}</h2>
          <p className="text-xs text-slate-500 mt-1">ID: {id}</p>
        </div>
        <div className="flex gap-2">
          <Link to={`/workflows/${id}/edit`} className="text-xs px-3 py-1.5 bg-slate-800 rounded hover:bg-slate-700">编辑</Link>
          <button onClick={() => api.runs.trigger(id!)} className="text-xs px-3 py-1.5 bg-emerald-700 rounded hover:bg-emerald-600">运行</button>
        </div>
      </div>

      <h3 className="font-semibold mb-2 text-slate-300">DAG 预览</h3>
      {graph ? (
        <DAGViewer graph={graph as GraphData} />
      ) : (
        <p className="text-slate-500 text-sm mb-4">暂无图数据</p>
      )}

      <h3 className="font-semibold mb-2 mt-6 text-slate-300">运行历史</h3>
      <div className="space-y-1">
        {runs && runs.length > 0 ? runs.map((r: RunRecord) => (
          <Link key={r.id} to={`/runs/${r.id}`} className="flex items-center justify-between p-2 bg-slate-900 rounded border border-slate-800 hover:bg-slate-800 text-sm">
            <span>{r.id.slice(0, 8)}... <span className="text-slate-500">{r.created_at}</span></span>
            <span className={`text-xs px-1 py-0.5 rounded ${
              r.status === 'completed' ? 'bg-emerald-900 text-emerald-300' :
              r.status === 'failed' ? 'bg-red-900 text-red-300' :
              r.status === 'running' ? 'bg-blue-900 text-blue-300' :
              'bg-slate-800 text-slate-400'
            }`}>{r.status}</span>
          </Link>
        )) : <p className="text-xs text-slate-500">暂无运行记录</p>}
      </div>
    </div>
  )
}
