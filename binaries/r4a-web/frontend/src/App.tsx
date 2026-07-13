import React from 'react';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AuthProvider, useAuth } from './context/AuthContext';
import Layout from './components/Layout';
import Login from './pages/Login';
import Dashboard from './pages/Dashboard';
import Git from './pages/Git';
import Vault from './pages/Vault';
import Manifests from './pages/Manifests';
import RBAC from './pages/RBAC';
import Updates from './pages/Updates';
import Containers from './pages/Containers';
import Connections from './pages/Connections';
import Logs from './pages/Logs';

const queryClient = new QueryClient();

const ProtectedRoute: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { isAuthenticated } = useAuth();
  
  if (!isAuthenticated) {
    return <Navigate to="/login" replace />;
  }
  
  return <>{children}</>;
};

function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <BrowserRouter>
          <Routes>
            <Route path="/login" element={<Login />} />
            
            <Route 
              path="/" 
              element={
                <ProtectedRoute>
                  <Layout />
                </ProtectedRoute>
              }
            >
              <Route index element={<Dashboard />} />
              <Route path="git" element={<Git />} />
              <Route path="manifests" element={<Manifests />} />
              <Route path="vault" element={<Vault />} />
              <Route path="rbac" element={<RBAC />} />
              <Route path="updates" element={<Updates />} />
              <Route path="containers" element={<Containers />} />
              <Route path="connections" element={<Connections />} />
              <Route path="logs" element={<Logs />} />
            </Route>
            
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </BrowserRouter>
      </AuthProvider>
    </QueryClientProvider>
  );
}

export default App;
