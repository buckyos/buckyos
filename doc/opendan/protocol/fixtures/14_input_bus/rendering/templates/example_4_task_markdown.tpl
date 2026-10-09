# Task
{{ session.objective }}

{% for msg in input.messages %}
{{ msg | render_format: "message.markdown" }}
{% endfor %}
