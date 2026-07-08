import { useQuery } from '@tanstack/react-query'
import { api } from '../lib/api'
import { Link } from 'react-router-dom'
import type { WorkflowSummary } from '../lib/types'

interface RunRecord {
  id: string
  workflow_id: string
  workflow_name?: string
  status: string
  created_at: string
}

export default function Dashboard() {
  const { data: workflows } = useQuery({ queryKey: ['workflows'], queryFn: api.workflows.list })
  const { data: runs } = useQuery({
    queryKey: ['runs'],
    queryFn: () => fetch('/api/runs').then<RunRecord[]>(r => r.json()).catch(() => []),
  })

  return (
    <div>
      <h2 className="text-xl font-bold mb-4">仪表盘</h2>
      <div className="grid grid-cols-2 gap-4 mb-6">
        <div className="p-4 bg-slate-900 rounded border border-slate-800">
          <p className="text-2xl font-bold text-white">{workflows?.length ?? '-'}</p>
          <p className="text-xs text-slate-400 mt-1">工作流</p>
        </div>
        <div className="p-4 bg-slate-900 rounded border border-slate-800">
          <p className="text-2xl font-bold text-white">{runs?.length ?? '-'}</p>
          <p className="text-xs text-slate-400 mt-1">运行记录</p>
        </div>
      </div>

      <h3 className="font-semibold mb-2 text-slate-300">最近更新</h3>
      <div className="space-y-1">
        {workflows?.slice(0, 5).map((w: WorkflowSummary) => (
          <Link key={w.id} to={`/workflows/${w.id}`} className="block p-2 bg-slate-900 rounded border border-slate-800 hover:bg-slate-800 text-sm">
            {w.name} <span className="text-slate-500">· {w.node_count} 节点</span>
          </Link>
        ))}
        {(!workflows || workflows.length === 0) && (
          <p className="text-xs text-slate-500">暂无工作流</p>
        )}
      </div>

      <h3 className="font-semibold mb-2 mt-6 text-slate-300">最近运行</h3>
      <div className="space-y-1">
        {runs?.slice(0, 5).map((r: RunRecord) => (
          <Link key={r.id} to={`/runs/${r.id}`} className="block p-2 bg-slate-900 rounded border border-slate-800 hover:bg-slate-800 text-sm">
            {r.workflow_name ?? r.workflow_id} <span className={`ml-2 text-xs px-1 py-0.5 rounded ${
              r.status === 'completed' ? 'bg-emerald-900 text-emerald-300' :
              r.status === 'failed' ? 'bg-red-900 text-red-300' :
              r.status === 'running' ? 'bg-blue-900 text-blue-300' :
              'bg-slate-800 text-slate-400'
            }`}>{r.status}</span>
          </Link>
        ))}
        {(!runs || runs.length === 0) && (
          <p className="text-xs text-slate-500">暂无运行记录</p>
        )}
      </div>
    </div>
  )
}
