import type { NodeStatus } from '../lib/types'

const colorMap: Record<NodeStatus, string> = {
  Pending: 'bg-slate-600',
  Running: 'bg-blue-500',
  Completed: 'bg-green-500',
  Failed: 'bg-red-500',
  TimedOut: 'bg-yellow-500',
}

export default function NodeStatusBadge({ status }: { status: NodeStatus }) {
  return (
    <span className={`inline-block w-2 h-2 rounded-full ${colorMap[status]}`} title={status} />
  )
}
