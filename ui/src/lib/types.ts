export type NodeStatus = 'Pending' | 'Running' | 'Completed' | 'Failed' | 'TimedOut'

export type WsMessage =
  | { type: 'node_status'; data: { node_id: string; status: NodeStatus; ts: number } }
  | { type: 'node_chunk'; data: { node_id: string; text: string; ts: number } }
  | { type: 'snapshot'; data: { running_count: number; elapsed_secs: number; nodes: Record<string, string> } }
  | { type: 'workflow_done'; data: { status: 'completed' | 'failed' | 'timeout'; duration_secs: number } }
  | { type: 'error'; data: { message: string } }

export interface WorkflowSummary {
  id: string; name: string; node_count: number; updated_at: string
}

export interface GraphNode {
  id: string; label: string; status?: NodeStatus | null
}

export interface GraphEdge {
  from: string; to: string; label: string; exit_reason?: string | null
}

export interface GraphData {
  nodes: GraphNode[]; edges: GraphEdge[]; dataflows: { from: string; to: string }[]
  running_count?: number; elapsed_secs?: number
}
