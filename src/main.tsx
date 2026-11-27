import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Toaster } from 'sonner';
import App from './App';
import { useAppStore } from './store';
import './styles.css';

function ThemedToaster() {
  const theme = useAppStore((s) => s.theme);
  return (
    <Toaster
      theme={theme}
      position="bottom-right"
      closeButton
      toastOptions={{
        style: {
          background: 'var(--color-modal)',
          border: '1px solid var(--color-line)',
          color: 'var(--color-ink)',
        },
      }}
    />
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
    <ThemedToaster />
  </StrictMode>,
);
