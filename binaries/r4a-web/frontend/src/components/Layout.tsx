import React from 'react';
import { Outlet, NavLink, useNavigate } from 'react-router-dom';
import { useAuth } from '../context/AuthContext';
import {
  LayoutDashboard,
  GitBranch,
  FileCode,
  Lock,
  Shield,
  RefreshCw,
  LogOut,
  Server,
  Container,
  Wifi,
  ScrollText
} from 'lucide-react';

const Layout: React.FC = () => {
  const { logout, user } = useAuth();
  const navigate = useNavigate();

  const handleLogout = () => {
    logout();
    navigate('/login');
  };

  const navItems = [
    { path: '/', label: 'Dashboard', icon: LayoutDashboard },
    { path: '/containers', label: 'Containers', icon: Container },
    { path: '/logs', label: 'Logs', icon: ScrollText },
    { path: '/git', label: 'Git', icon: GitBranch },
    { path: '/manifests', label: 'Manifests', icon: FileCode },
    { path: '/vault', label: 'Vault', icon: Lock },
    { path: '/rbac', label: 'RBAC', icon: Shield },
    { path: '/updates', label: 'Updates', icon: RefreshCw },
    { path: '/connections', label: 'Connections', icon: Wifi },
  ];

  return (
    <div className="flex h-screen bg-[#0b0c10] text-gray-300 font-sans">
      <aside className="w-64 bg-[#1f2833] border-r border-gray-800 flex flex-col">
        <div className="p-6 flex items-center gap-3 border-b border-gray-800">
          <div className="w-8 h-8 bg-[#0b0c10] rounded flex items-center justify-center border border-[#66fcf1]/30">
            <Server className="w-4 h-4 text-[#66fcf1]" />
          </div>
          <span className="font-bold text-white tracking-wide">r4a cluster</span>
        </div>

        <nav className="flex-1 py-6 px-4 space-y-2">
          {navItems.map((item) => (
            <NavLink
              key={item.path}
              to={item.path}
              className={({ isActive }) =>
                `flex items-center gap-3 px-4 py-3 rounded transition-colors ${
                  isActive
                    ? 'bg-[#0b0c10] text-[#66fcf1] border-l-2 border-[#66fcf1]'
                    : 'hover:bg-[#0b0c10]/50 hover:text-white'
                }`
              }
            >
              <item.icon className="w-5 h-5" />
              <span className="font-medium">{item.label}</span>
            </NavLink>
          ))}
        </nav>

        <div className="p-4 border-t border-gray-800">
          <div className="mb-4 px-4">
            <p className="text-xs text-gray-500 uppercase tracking-wider">Logged in as</p>
            <p className="text-sm text-white truncate">{user?.username || 'Admin'}</p>
          </div>
          <button
            onClick={handleLogout}
            className="flex items-center gap-3 px-4 py-2 w-full text-left text-gray-400 hover:text-white hover:bg-[#0b0c10]/50 rounded transition-colors"
          >
            <LogOut className="w-5 h-5" />
            <span>Logout</span>
          </button>
        </div>
      </aside>

      <main className="flex-1 overflow-auto">
        <div className="p-8">
          <Outlet />
        </div>
      </main>
    </div>
  );
};

export default Layout;
