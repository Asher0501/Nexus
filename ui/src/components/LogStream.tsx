import { useEffect, useRef } from 'react'

interface LogEntry {
  nodeId: string
  text: string
  ts: number
}

interface Props {
  logs: LogEntry[]
}

export default function LogStream({ logs }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null)
  useEffect(() => { bottomRef.current?.scrollIntoView({ behavior: 'smooth' }) }, [logs.length])

  if (logs.length === 0) {
    return (
      <div className="h-64 overflow-auto bg-slate-950 rounded border border-slate-800 p-3 font-mono text-xs text-slate-500 flex items-center justify-center">
        等待日志...
      </div>
    )
  }

  return (
    <div className="h-64 overflow-auto bg-slate-950 rounded border border-slate-800 p-3 font-mono text-xs">
      {logs.map((log, i) => (
        <div key={i} className="py-0.5">
          <span className="text-slate-500">[{log.ts}]</span>
          <span className="text-emerald-400 ml-2">{log.nodeId}:</span>
          <span className="text-slate-300 ml-1">{log.text}</span>
        </div>
      ))}
      <div ref={bottomRef} />
    </div>
  )
}
