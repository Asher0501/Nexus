import { useQuery } from '@tanstack/react-query'
import { api } from '../lib/api'

export function useWorkflows() {
  return useQuery({ queryKey: ['workflows'], queryFn: api.workflows.list })
}

export function useWorkflowGraph(id: string) {
  return useQuery({ queryKey: ['workflow-graph', id], queryFn: () => api.workflows.graph(id) })
}
