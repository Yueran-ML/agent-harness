import ReactDOM from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import App from './app/App';
import { initialSize } from './size';
import { initialTheme } from './themes';
import './ui/tokens.css';
import './ui/base.css';
import './ui/shell.css';
import './ui/conversation.css';
import './ui/composer.css';

// The theme and the size of type are on the page before anything is drawn on it.
document.documentElement.dataset.theme = initialTheme();
document.documentElement.dataset.size = initialSize();

ReactDOM.createRoot(document.getElementById('root')!).render(<App />);
