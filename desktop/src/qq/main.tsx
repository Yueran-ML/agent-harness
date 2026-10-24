import ReactDOM from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import { initialSize } from '../size';
import '../ui/tokens.css';
import '../ui/base.css';
import '../key/key.css';
import '../key/steps.css';
import { QqApp } from './QqApp';

// The size of type that was chosen in the main window holds here too.
document.documentElement.dataset.size = initialSize();

ReactDOM.createRoot(document.getElementById('root')!).render(<QqApp />);
