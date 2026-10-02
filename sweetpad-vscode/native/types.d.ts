// Public N-API contract, available before compiling the Mac-only addon.
// Keep this in sync with src/lib.rs when changing the bindings.
export interface XcodeVersion {
  developerDir: string;
  shortVersion: string;
  buildVersion: string;
  majorVersion: number;
}
export interface ProjectInfo {
  name: string;
  targets: string[];
  configurations: string[];
  schemes: string[];
}
export interface WorkspaceInfo {
  name: string;
  projects: string[];
  packages: string[];
  schemes: string[];
}
export interface BuildSettingsOptions {
  project?: string | null;
  workspace?: string | null;
  scheme?: string | null;
  target?: string | null;
  configuration: string;
  sdk?: string | null;
  arch?: string | null;
  destination?: string | null;
  xcconfig?: string | null;
  xcode?: string | null;
  derivedDataPath?: string | null;
  keys?: string[] | null;
}
export interface TargetBuildSettings {
  target: string;
  settings: Record<string, string>;
}
export interface CompilerToolInvocation {
  tool: string;
  arguments: string[];
  inputFiles: string[];
}
export interface TargetCompilerArguments {
  target: string;
  swift?: CompilerToolInvocation | null;
  clang?: CompilerToolInvocation | null;
  link?: CompilerToolInvocation | null;
}
export interface SchemeBuildable {
  blueprintName: string;
  blueprintIdentifier: string;
  buildableName: string;
  container: string;
}
export interface SchemeBuildEntry {
  buildable: SchemeBuildable;
  forRunning: boolean;
  forTesting: boolean;
  forProfiling: boolean;
  forArchiving: boolean;
  forAnalyzing: boolean;
}
export interface SchemeTestable {
  buildable: SchemeBuildable;
  skipped: boolean;
}
export interface SchemeTestAction {
  configuration: string;
  testables: SchemeTestable[];
  codeCoverageEnabled: boolean;
}
export interface SchemeCommandLineArgument {
  argument: string;
  isEnabled: boolean;
}
export interface SchemeEnvironmentVariable {
  key: string;
  value?: string;
  isEnabled: boolean;
}
export interface SchemeInfo {
  buildEntries: SchemeBuildEntry[];
  buildImplicitDependencies: boolean;
  parallelizeBuildables: boolean;
  testAction?: SchemeTestAction | null;
  launchTarget?: SchemeBuildable | null;
  launchConfiguration?: string | null;
  profileConfiguration?: string | null;
  archiveConfiguration?: string | null;
  analyzeConfiguration?: string | null;
  launchArguments: SchemeCommandLineArgument[];
  launchEnvironmentVariables: SchemeEnvironmentVariable[];
  launchLanguage?: string | null;
  launchRegion?: string | null;
}
export declare function xcodeVersion(developerDir?: string | null): XcodeVersion;
export declare function flushXcodeCache(): void;
export declare function listProject(path: string): ProjectInfo;
export declare function listWorkspace(path: string): WorkspaceInfo;
export declare function schemes(path: string): Promise<string[]>;
export declare function targets(path: string): Promise<string[]>;
export declare function configurations(path: string): string[];
export declare function buildSettings(options: BuildSettingsOptions): TargetBuildSettings[];
export declare function compilerArguments(options: BuildSettingsOptions): TargetCompilerArguments[];
export declare function parseScheme(path: string): SchemeInfo;
