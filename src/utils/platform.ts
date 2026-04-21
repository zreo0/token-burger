import { invoke } from '@tauri-apps/api/core';
import type { PlatformInfo } from '../types';

export async function getPlatformInfo(): Promise<PlatformInfo> {
    return invoke<PlatformInfo>('get_platform_info');
}
