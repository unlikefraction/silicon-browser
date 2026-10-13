import type { Usage } from './types';

export const date = (value: string) => new Date(value).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
export const cost = (value?: Usage) => value?.cost?.total ? `${(value.cost.total.micros / 1e6).toFixed(4)} ${value.cost.total.currency}` : 'Pending';
export const bytes = (value: number) => value >= 1e9 ? `${(value / 1e9).toFixed(2)} GB` : `${(value / 1e6).toFixed(1)} MB`;
export const splitAccess = (value: string) => value.split(',').map(item => item.trim()).filter(Boolean);
/** Browser time in minutes with one decimal, as shown on usage rows and totals. */
export const minutes = (seconds: number) => (seconds / 60).toFixed(1);
/** A status for display: `in_progress` reads as `in progress`. */
export const statusLabel = (status: string) => status.replaceAll('_', ' ');
