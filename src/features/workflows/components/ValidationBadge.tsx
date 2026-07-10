import { AlertTriangle, CheckCircle2, XCircle } from 'lucide-react';
import { Badge } from '../../../components/ui/Badge';
import type { ValidationStatus } from '../../../types/workflow';

export function ValidationBadge({ status }: { status: ValidationStatus }) {
  if (status === 'errors') {
    return (
      <Badge variant="danger">
        <XCircle size={10} aria-hidden="true" />
        Errors
      </Badge>
    );
  }
  if (status === 'warnings') {
    return (
      <Badge variant="info">
        <AlertTriangle size={10} aria-hidden="true" />
        Warnings
      </Badge>
    );
  }
  return (
    <Badge variant="primary">
      <CheckCircle2 size={10} aria-hidden="true" />
      Valid
    </Badge>
  );
}
