import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import { backendOrigin } from './backend-origin.mjs';
export default defineConfig({plugins:[solid()],define:{'import.meta.env.SB_BACKEND_ORIGIN':JSON.stringify(backendOrigin(process.env.SB_PUBLIC_BACKEND_URL))},server:{host:'127.0.0.1',port:8092,strictPort:true},build:{sourcemap:false}});
