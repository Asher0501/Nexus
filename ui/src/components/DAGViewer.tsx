import { useEffect, useRef } from 'react'
import cytoscape from 'cytoscape'
import type { GraphData } from '../lib/types'

interface Props {
  graph: GraphData | null
  onNodeClick?: (nodeId: string) => void
}

export default function DAGViewer({ graph, onNodeClick }: Props) {
  const containerRef = useRef<HTMLDivElement>(null)
  const cyRef = useRef<cytoscape.Core | null>(null)

  useEffect(() => {
    if (!containerRef.current || !graph) return
    if (cyRef.current) {
      cyRef.current.destroy()
      cyRef.current = null
    }
    const cy = cytoscape({
      container: containerRef.current,
      elements: [
        ...graph.nodes.map(n => ({
          data: { id: n.id, label: n.label },
          classes: n.status || 'Pending',
        })),
        ...graph.edges.map((e, i) => ({
          data: { id: `e${i}`, source: e.from, target: e.to, label: e.label },
        })),
      ],
      style: [
        {
          selector: 'node',
          style: {
            label: 'data(label)',
            'text-valign': 'center',
            'text-halign': 'center',
            color: '#fff',
            'font-size': '12px',
            width: 60,
            height: 40,
            'border-width': 2,
            'border-color': '#334155',
            'background-color': '#64748b',
          },
        },
        { selector: 'node.Completed', style: { 'background-color': '#22c55e', 'border-color': '#16a34a' } },
        { selector: 'node.Running', style: { 'background-color': '#3b82f6', 'border-color': '#2563eb' } },
        { selector: 'node.Failed', style: { 'background-color': '#ef4444', 'border-color': '#dc2626' } },
        { selector: 'node.TimedOut', style: { 'background-color': '#f59e0b', 'border-color': '#d97706' } },
        {
          selector: 'edge',
          style: {
            width: 2,
            'line-color': '#475569',
            'target-arrow-color': '#475569',
            'target-arrow-shape': 'triangle',
            'curve-style': 'bezier',
            label: 'data(label)',
            'font-size': '10px',
            color: '#94a3b8',
          },
        },
      ],
      layout: { name: 'dagre', rankDir: 'LR', spacingFactor: 1.5 } as cytoscape.LayoutOptions,
    })
    cyRef.current = cy
    cy.on('tap', 'node', (e) => onNodeClick?.(e.target.id()))
    return () => {
      cy.destroy()
      cyRef.current = null
    }
  }, [graph])

  // update node styles when graph data updates (from WS)
  useEffect(() => {
    if (!cyRef.current || !graph) return
    for (const n of graph.nodes) {
      const el = cyRef.current.getElementById(n.id)
      if (el.length) {
        el.classes(n.status || 'Pending')
      }
    }
  }, [graph?.nodes])

  return <div ref={containerRef} className="w-full h-80 bg-slate-950 rounded border border-slate-800" />
}
