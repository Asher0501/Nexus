import { useQuery, useMutation } from '@tanstack/react-query'
import { api } from '../lib/api'

export function useRunGraph(runId: string) {
  return useQuery({ queryKey: ['run-graph', runId], queryFn: () => api.runs.graphStatus(runId) })
}

export function useTriggerRun() {
  return useMutation({ mutationFn: (wfId: string) => api.runs.trigger(wfId) })
}
