import './lan.css';
import { mountLanApp } from './boot';

const container = document.getElementById('root');
if (container) mountLanApp(container);
