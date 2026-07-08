import { ReactNode } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { cn } from '../lib/utils'

const nav = [
  { to: '/', label: '仪表盘' },
  { to: '/workflows', label: '工作流' },
  { to: '/runs', label: '运行记录' },
]

export default function Layout({ children }: { children: ReactNode }) {
  const loc = useLocation()
  return (
    <div className="flex h-screen bg-slate-950 text-slate-200">
      <aside className="w-56 border-r border-slate-800 p-4 flex flex-col gap-2">
        <h1 className="text-lg font-bold mb-4 text-white">Nexus</h1>
        {nav.map(n => (
          <Link key={n.to} to={n.to} className={cn('px-3 py-2 rounded text-sm hover:bg-slate-800', loc.pathname === n.to && 'bg-slate-800 text-white')}>{n.label}</Link>
        ))}
      </aside>
      <main className="flex-1 overflow-auto p-6">{children}</main>
    </div>
  )
}
