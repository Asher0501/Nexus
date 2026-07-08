import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '../lib/api'
import { Link } from 'react-router-dom'
import type { WorkflowSummary } from '../lib/types'

export default function Workflows() {
  const qc = useQueryClient()
  const { data: workflows, isLoading } = useQuery({ queryKey: ['workflows'], queryFn: api.workflows.list })
  const triggerMut = useMutation({
    mutationFn: (id: string) => api.runs.trigger(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['runs'] }),
  })

  return (
    <div>
      <h2 className="text-xl font-bold mb-4">工作流</h2>
      {isLoading && <p className="text-slate-400">加载中...</p>}
      <div className="space-y-2">
        {workflows?.map((w: WorkflowSummary) => (
          <div key={w.id} className="flex items-center justify-between p-3 bg-slate-900 rounded border border-slate-800">
            <div>
              <Link to={`/workflows/${w.id}`} className="text-white font-medium hover:text-blue-400">{w.name}</Link>
              <p className="text-xs text-slate-500 mt-1">{w.node_count} 个节点 · {w.updated_at}</p>
            </div>
            <div className="flex gap-2">
              <Link to={`/workflows/${w.id}/edit`} className="text-xs px-2 py-1 bg-slate-800 rounded hover:bg-slate-700">编辑</Link>
              <button onClick={() => triggerMut.mutate(w.id)} disabled={triggerMut.isPending} className="text-xs px-2 py-1 bg-emerald-700 rounded hover:bg-emerald-600 disabled:opacity-50">
                {triggerMut.isPending ? '运行中...' : '运行'}
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
