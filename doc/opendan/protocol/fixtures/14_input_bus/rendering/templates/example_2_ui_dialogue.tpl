{% if session.background_hint_changed %}
<background_environment current_clock="{{ runtime.clock_text }}">
{{ session.default_changed_background_hint_text }}
</background_environment>
{% endif %}
{{ session.current_todo | render_format: "todo.summary_xml" }}
{{ input | render_format: "input.xml" }}
