use super::process::InstanceLaunchParams;

pub(super) fn build_launch_args(
    backend: &str,
    deploy_type: &str,
    model_path: &str,
    params: &InstanceLaunchParams,
) -> Result<Vec<String>, String> {
    if deploy_type == "script" {
        let mut args = vec![
            "start".to_string(),
            "--model".to_string(),
            model_path.to_string(),
            "--host".to_string(),
            params.host.clone(),
            "--port".to_string(),
            params.port.to_string(),
        ];
        args.extend(params.extra_args.clone());
        return Ok(args);
    }
    match backend {
        "llama_cpp" => {
            let mut args = vec![
                "-m".to_string(),
                model_path.to_string(),
                "--host".to_string(),
                params.host.clone(),
                "--port".to_string(),
                params.port.to_string(),
            ];
            if let Some(ctx_size) = params.ctx_size {
                args.extend(["--ctx-size".to_string(), ctx_size.to_string()]);
            }
            if let Some(gpu_layers) = params.gpu_layers {
                // -1 = platform default — leave GPU layer detection to llama.cpp's runtime environment
                //  0 = user explicit CPU-only debug
                // >0 = user explicit value
                if gpu_layers >= 0 {
                    args.extend(["--n-gpu-layers".to_string(), gpu_layers.to_string()]);
                }
            }
            if let Some(threads) = params.threads {
                args.extend(["--threads".to_string(), threads.to_string()]);
            }
            args.extend(params.extra_args.clone());
            Ok(args)
        }
        "ollama" | "vllm" | "lmdeploy" | "mindie" | "custom" | "triton" => {
            let mut args = vec![
                "--model".to_string(),
                model_path.to_string(),
                "--host".to_string(),
                params.host.clone(),
                "--port".to_string(),
                params.port.to_string(),
            ];
            args.extend(params.extra_args.clone());
            Ok(args)
        }
        _ => Err("backend type not supported".to_string()),
    }
}

pub(super) fn command_summary(program: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(program.to_string());
    parts.extend(args.iter().map(|arg| sanitize_arg_for_display(arg)));
    serde_json::to_string(&parts).unwrap_or_else(|_| "[\"<command>\"]".to_string())
}

fn sanitize_arg_for_display(arg: &str) -> String {
    let lower = arg.to_ascii_lowercase();
    if [
        "token",
        "secret",
        "password",
        "api-key",
        "api_key",
        "authorization",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        "[redacted]".to_string()
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_params(gpu_layers: Option<i64>) -> InstanceLaunchParams {
        InstanceLaunchParams {
            host: "0.0.0.0".to_string(),
            port: 8080,
            ctx_size: Some(4096),
            gpu_layers,
            threads: Some(4),
            extra_args: vec![],
        }
    }

    #[test]
    fn gpu_layers_none_omits_flag() {
        let p = make_params(None);
        let args = build_launch_args("llama_cpp", "local", "/models/test.gguf", &p).unwrap();
        let joined = args.join(" ");
        assert!(!joined.contains("--n-gpu-layers"));
        assert!(joined.contains("-m /models/test.gguf"));
        assert!(joined.contains("--ctx-size 4096"));
    }

    #[test]
    fn gpu_layers_negative_one_omits_flag() {
        let p = make_params(Some(-1));
        let args = build_launch_args("llama_cpp", "local", "/models/test.gguf", &p).unwrap();
        let joined = args.join(" ");
        assert!(!joined.contains("--n-gpu-layers"));
    }

    #[test]
    fn gpu_layers_zero_passes_flag() {
        let p = make_params(Some(0));
        let args = build_launch_args("llama_cpp", "local", "/models/test.gguf", &p).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("--n-gpu-layers 0"));
    }

    #[test]
    fn gpu_layers_positive_passes_flag() {
        let p = make_params(Some(35));
        let args = build_launch_args("llama_cpp", "local", "/models/test.gguf", &p).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("--n-gpu-layers 35"));
    }

    #[test]
    fn script_deploy_type_does_not_add_backend_flags() {
        let p = make_params(Some(10));
        let args = build_launch_args("llama_cpp", "script", "/models/test.gguf", &p).unwrap();
        let joined = args.join(" ");
        assert!(joined.starts_with("start "));
        assert!(!joined.contains("--n-gpu-layers"));
    }

    #[test]
    fn command_summary_shows_sanitized_args() {
        let summary = command_summary(
            "llama-server",
            &["-m".to_string(), "/models/test.gguf".to_string()],
        );
        assert!(summary.contains("llama-server"));
        assert!(summary.contains("/models/test.gguf"));
    }
}
