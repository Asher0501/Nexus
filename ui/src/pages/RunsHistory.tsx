import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'

interface RunRecord {
  id: string
  workflow_id: string
  workflow_name?: string
  status: string
  created_at: string
}

export default function RunsHistory() {
  const { data: runs, isLoading } = useQuery({
    queryKey: ['runs'],
    queryFn: () => fetch('/api/runs').then<RunRecord[]>(r => r.json()).catch(() => []),
  })

  return (
    <div>
      <h2 className="text-xl font-bold mb-4">运行记录</h2>
      {isLoading && <p className="text-slate-400">加载中...</p>}
      <div className="space-y-1">
        {runs?.map((r: RunRecord) => (
          <Link key={r.id} to={`/runs/${r.id}`} className="flex items-center justify-between p-3 bg-slate-900 rounded border border-slate-800 hover:bg-slate-800 text-sm">
            <div>
              <span className="text-white">{r.workflow_name ?? r.workflow_id}</span>
              <span className="text-slate-500 ml-2">{r.id.slice(0, 8)}...</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-xs text-slate-500">{r.created_at}</span>
              <span className={`text-xs px-1 py-0.5 rounded ${
                r.status === 'completed' ? 'bg-emerald-900 text-emerald-300' :
                r.status === 'failed' ? 'bg-red-900 text-red-300' :
                r.status === 'running' ? 'bg-blue-900 text-blue-300' :
                'bg-slate-800 text-slate-400'
              }`}>{r.status}</span>
            </div>
          </Link>
        ))}
        {runs && runs.length === 0 && (
          <p className="text-xs text-slate-500">暂无运行记录</p>
        )}
      </div>
    </div>
  )
}
