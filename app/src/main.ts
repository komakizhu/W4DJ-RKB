import { bindApp } from './app';
import './styles.css';

const root = document.querySelector<HTMLElement>('#app');

if (root) {
  bindApp(root);
}
