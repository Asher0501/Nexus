import { Routes, Route } from 'react-router-dom'
import Layout from './components/Layout'
import Dashboard from './pages/Dashboard'
import Workflows from './pages/Workflows'
import WorkflowDetail from './pages/WorkflowDetail'
import WorkflowEditor from './pages/WorkflowEditor'
import RunMonitor from './pages/RunMonitor'
import RunsHistory from './pages/RunsHistory'

export default function App() {
  return (
    <Layout>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/workflows" element={<Workflows />} />
        <Route path="/workflows/:id" element={<WorkflowDetail />} />
        <Route path="/workflows/:id/edit" element={<WorkflowEditor />} />
        <Route path="/runs" element={<RunsHistory />} />
        <Route path="/runs/:id" element={<RunMonitor />} />
      </Routes>
    </Layout>
  )
}
