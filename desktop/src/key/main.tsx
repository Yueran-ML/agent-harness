import ReactDOM from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import '../ui/tokens.css';
import '../ui/base.css';
import './key.css';
import { KeyApp } from './KeyApp';

ReactDOM.createRoot(document.getElementById('root')!).render(<KeyApp />);
