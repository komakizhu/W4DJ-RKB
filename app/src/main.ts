import { invoke } from '@tauri-apps/api/core';
import { bindApp } from './app';
import type { AppInfo } from './app';
import './styles.css';

const root = document.querySelector<HTMLElement>('#app');

if (root) {
  void invoke<AppInfo>('app_info')
    .then((info) => bindApp(root, undefined, undefined, info.product_name))
    .catch(() => bindApp(root));
}
