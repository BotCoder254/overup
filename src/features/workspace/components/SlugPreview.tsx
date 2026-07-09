interface SlugPreviewProps {
  slug: string;
}

/**
 * Live URL preview under the workspace name field. Feedback only — the
 * backend generates the canonical slug and may adjust it for reserved
 * words or collisions.
 */
export function SlugPreview({ slug }: SlugPreviewProps) {
  return (
    <p aria-live="polite" className="text-sm text-steel">
      {slug ? (
        <>
          Your workspace URL: <span className="font-mono text-charcoal">/w/{slug}</span>
          <span className="block text-xs">Preview — the final address may differ.</span>
        </>
      ) : (
        'Your workspace URL will appear here as you type.'
      )}
    </p>
  );
}
