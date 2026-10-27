import ReactDOM from 'react-dom/client';
import '@fontsource-variable/figtree';
import '@fontsource-variable/jetbrains-mono';
import { initialSize } from '../size';
import '../ui/tokens.css';
import '../ui/base.css';
import './key.css';
import { KeyApp } from './KeyApp';

// The size of type that was chosen in the main window holds here too.
document.documentElement.dataset.size = initialSize();

ReactDOM.createRoot(document.getElementById('root')!).render(<KeyApp />);
