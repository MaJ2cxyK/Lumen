import { lazy, Suspense } from 'react';
import { HashRouter, Navigate, Route, Routes } from 'react-router-dom';
import Layout from './components/Layout';
import { Loading } from './components/ui';
import { I18nProvider, useI18n } from './lib/i18n';

// 路由级代码分割：首屏只加载总览，其余页面按需加载。
const Dashboard = lazy(() => import('./pages/Dashboard'));
const Projects = lazy(() => import('./pages/Projects'));
const ProjectDetail = lazy(() => import('./pages/ProjectDetail'));
const Agents = lazy(() => import('./pages/Agents'));
const Extensions = lazy(() => import('./pages/Extensions'));
const Teams = lazy(() => import('./pages/Teams'));
const Tasks = lazy(() => import('./pages/Tasks'));
const TaskDetail = lazy(() => import('./pages/TaskDetail'));
const Usage = lazy(() => import('./pages/Usage'));
const Budgets = lazy(() => import('./pages/Budgets'));
const Approvals = lazy(() => import('./pages/Approvals'));
const History = lazy(() => import('./pages/History'));
const Settings = lazy(() => import('./pages/Settings'));

function PageFallback() {
  return (
    <div className="flex h-full items-center justify-center">
      <Loading />
    </div>
  );
}

export default function App() {
  return (
    <I18nProvider>
      <AppRoutes />
    </I18nProvider>
  );
}

function AppRoutes() {
  const { lang } = useI18n();
  return (
    <HashRouter key={lang}>
      <Routes>
        <Route element={<Layout />}>
          <Route
            index
            element={
              <Suspense fallback={<PageFallback />}>
                <Dashboard />
              </Suspense>
            }
          />
          <Route
            path="projects"
            element={
              <Suspense fallback={<PageFallback />}>
                <Projects />
              </Suspense>
            }
          />
          <Route
            path="projects/:id"
            element={
              <Suspense fallback={<PageFallback />}>
                <ProjectDetail />
              </Suspense>
            }
          />
          <Route
            path="agents"
            element={
              <Suspense fallback={<PageFallback />}>
                <Agents />
              </Suspense>
            }
          />
          <Route
            path="settings/extensions"
            element={
              <Suspense fallback={<PageFallback />}>
                <Extensions />
              </Suspense>
            }
          />
          <Route
            path="settings/teams"
            element={
              <Suspense fallback={<PageFallback />}>
                <Teams />
              </Suspense>
            }
          />
          <Route
            path="tasks"
            element={
              <Suspense fallback={<PageFallback />}>
                <Tasks />
              </Suspense>
            }
          />
          <Route
            path="tasks/:id"
            element={
              <Suspense fallback={<PageFallback />}>
                <TaskDetail />
              </Suspense>
            }
          />
          <Route
            path="settings/usage"
            element={
              <Suspense fallback={<PageFallback />}>
                <Usage />
              </Suspense>
            }
          />
          <Route
            path="settings/budgets"
            element={
              <Suspense fallback={<PageFallback />}>
                <Budgets />
              </Suspense>
            }
          />
          <Route
            path="approvals"
            element={
              <Suspense fallback={<PageFallback />}>
                <Approvals />
              </Suspense>
            }
          />
          <Route
            path="history"
            element={
              <Suspense fallback={<PageFallback />}>
                <History />
              </Suspense>
            }
          />
          <Route
            path="settings"
            element={
              <Suspense fallback={<PageFallback />}>
                <Settings />
              </Suspense>
            }
          />
          <Route path="extensions" element={<Navigate to="/settings/extensions" replace />} />
          <Route path="teams" element={<Navigate to="/settings/teams" replace />} />
          <Route path="usage" element={<Navigate to="/settings/usage" replace />} />
          <Route path="budgets" element={<Navigate to="/settings/budgets" replace />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </HashRouter>
  );
}
