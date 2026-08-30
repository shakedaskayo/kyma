{{- define "pensieve-engine.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "pensieve-engine.fullname" -}}
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

{{- define "pensieve-engine.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
app.kubernetes.io/name: {{ include "pensieve-engine.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
{{- end -}}

{{- define "pensieve-engine.selectorLabels" -}}
app.kubernetes.io/name: {{ include "pensieve-engine.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "pensieve-engine.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "pensieve-engine.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- /* Selector labels for one role deployment (adds pensieve.io/role). Arg: dict
       "ctx" $ "role" "<role>". */ -}}
{{- define "pensieve-engine.roleSelectorLabels" -}}
{{ include "pensieve-engine.selectorLabels" .ctx }}
pensieve.io/role: {{ .role }}
{{- end -}}

{{- /* The shared engine container for a role deployment, with PENSIEVE_ROLE injected
       ahead of the configured env (which the engine's role_components() honors).
       Arg: dict "ctx" $ "role" "<role>". */ -}}
{{- define "pensieve-engine.roleContainer" -}}
{{- $ctx := .ctx -}}
- name: pensieve-engine
  image: "{{ $ctx.Values.image.repository }}:{{ $ctx.Values.image.tag }}"
  imagePullPolicy: {{ $ctx.Values.image.pullPolicy }}
  ports:
    - name: http
      containerPort: 8080
      protocol: TCP
  env:
    - name: PENSIEVE_ROLE
      value: {{ .role | quote }}
    {{- range $k, $v := $ctx.Values.env }}
    - name: {{ $k }}
      value: {{ $v | quote }}
    {{- end }}
  {{- if $ctx.Values.secretEnv }}
  envFrom:
    - secretRef:
        name: {{ include "pensieve-engine.fullname" $ctx }}-env
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
