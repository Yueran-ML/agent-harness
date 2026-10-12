import ReactDOM from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import App from './app/App';
import { initialTheme } from './themes';
import './ui/tokens.css';
import './ui/base.css';
import './ui/shell.css';
import './ui/conversation.css';
import './ui/composer.css';

// The theme is on the page before anything is drawn on it.
document.documentElement.dataset.theme = initialTheme();

ReactDOM.createRoot(document.getElementById('root')!).render(<App />);
