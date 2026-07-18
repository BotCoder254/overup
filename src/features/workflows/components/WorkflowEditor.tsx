import Editor, { type Monaco, type OnMount } from '@monaco-editor/react';
import type { editor } from 'monaco-editor';
import { useEffect, useRef } from 'react';
import { Spinner } from '../../../components/ui/Spinner';
import type { Diagnostic } from '../../../types/workflow';

interface WorkflowEditorProps {
  value: string;
  onChange: (value: string) => void;
  diagnostics: Diagnostic[];
  /** Line to scroll to and highlight (job/diagnostic selection). */
  revealLine?: number | null;
}

const MARKER_OWNER = 'overup-validation';

/** Editor colors from the shell palette — flat, dark, no gradients. */
function defineTheme(monaco: Monaco) {
  monaco.editor.defineTheme('overup', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      { token: 'type', foreground: '9d8cff' },
      { token: 'string.yaml', foreground: '4d9fff' },
      { token: 'number', foreground: '9d8cff' },
      { token: 'comment', foreground: '8f8d88' },
      { token: 'keyword', foreground: '9d8cff' },
    ],
    colors: {
      'editor.background': '#000000',
      'editor.foreground': '#e8e6e3',
      'editor.lineHighlightBackground': '#161614',
      'editorLineNumber.foreground': '#8f8d88',
      'editorLineNumber.activeForeground': '#e8e6e3',
      'editorIndentGuide.background': '#26251f',
      'editor.selectionBackground': '#6a59e845',
      'focusBorder': '#6a59e8',
    },
  });
}

/**
 * Monaco YAML editor for workflow files. Interactive — every keystroke feeds
 * the debounced server-side validator — but nothing is written back to
 * GitHub in this phase.
 */
export function WorkflowEditor({ value, onChange, diagnostics, revealLine }: WorkflowEditorProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Monaco | null>(null);

  const onMount: OnMount = (editorInstance, monaco) => {
    editorRef.current = editorInstance;
    monacoRef.current = monaco;
  };

  // Mirror server diagnostics as squiggles in the gutter.
  useEffect(() => {
    const monaco = monacoRef.current;
    const model = editorRef.current?.getModel();
    if (!monaco || !model) return;
    monaco.editor.setModelMarkers(
      model,
      MARKER_OWNER,
      diagnostics.map((diagnostic) => {
        const line = Math.min(Math.max(diagnostic.line ?? 1, 1), model.getLineCount());
        return {
          startLineNumber: line,
          endLineNumber: line,
          startColumn: 1,
          endColumn: model.getLineMaxColumn(line),
          message: diagnostic.path
            ? `${diagnostic.message} (${diagnostic.path})`
            : diagnostic.message,
          severity:
            diagnostic.severity === 'error'
              ? monaco.MarkerSeverity.Error
              : monaco.MarkerSeverity.Warning,
        };
      }),
    );
  }, [diagnostics, value]);

  useEffect(() => {
    if (!revealLine || !editorRef.current) return;
    editorRef.current.revealLineInCenter(revealLine);
    editorRef.current.setPosition({ lineNumber: revealLine, column: 1 });
    editorRef.current.focus();
  }, [revealLine]);

  return (
    <Editor
      language="yaml"
      theme="overup"
      value={value}
      onChange={(next) => onChange(next ?? '')}
      beforeMount={defineTheme}
      onMount={onMount}
      loading={
        <div className="flex h-full items-center justify-center">
          <Spinner className="h-5 w-5 text-steel" />
        </div>
      }
      options={{
        minimap: { enabled: false },
        fontSize: 13,
        fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
        scrollBeyondLastLine: false,
        wordWrap: 'on',
        tabSize: 2,
        renderWhitespace: 'boundary',
        padding: { top: 12, bottom: 12 },
        automaticLayout: true,
        folding: true,
        bracketPairColorization: { enabled: false },
      }}
    />
  );
}
