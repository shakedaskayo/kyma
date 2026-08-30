{{- define "kyma-engine.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "kyma-engine.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "kyma-engine.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
app.kubernetes.io/name: {{ include "kyma-engine.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
{{- end -}}

{{- define "kyma-engine.selectorLabels" -}}
app.kubernetes.io/name: {{ include "kyma-engine.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "kyma-engine.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "kyma-engine.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- /* Selector labels for one role deployment (adds kyma.io/role). Arg: dict
       "ctx" $ "role" "<role>". */ -}}
{{- define "kyma-engine.roleSelectorLabels" -}}
{{ include "kyma-engine.selectorLabels" .ctx }}
kyma.io/role: {{ .role }}
{{- end -}}

{{- /* The shared engine container for a role deployment, with KYMA_ROLE injected
       ahead of the configured env (which the engine's role_components() honors).
       Arg: dict "ctx" $ "role" "<role>". */ -}}
{{- define "kyma-engine.roleContainer" -}}
{{- $ctx := .ctx -}}
- name: kyma-engine
  image: "{{ $ctx.Values.image.repository }}:{{ $ctx.Values.image.tag }}"
  imagePullPolicy: {{ $ctx.Values.image.pullPolicy }}
  ports:
    - name: http
      containerPort: 8080
      protocol: TCP
  env:
    - name: KYMA_ROLE
      value: {{ .role | quote }}
    {{- range $k, $v := $ctx.Values.env }}
    - name: {{ $k }}
      value: {{ $v | quote }}
    {{- end }}
  {{- if $ctx.Values.secretEnv }}
  envFrom:
    - secretRef:
        name: {{ include "kyma-engine.fullname" $ctx }}-env
  {{- end }}
  livenessProbe:
    httpGet:
      path: /health
      port: http
    initialDelaySeconds: 15
    periodSeconds: 20
  readinessProbe:
    httpGet:
      path: /health
      port: http
    initialDelaySeconds: 5
    periodSeconds: 10
  resources:
    {{- toYaml $ctx.Values.resources | nindent 4 }}
{{- end -}}
