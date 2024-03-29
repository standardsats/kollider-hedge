{ containerTag ? "latest"
, prefixName ? ""
}:
let
  sources = import ./sources.nix;
  pkgs = import sources.nixpkgs {};
  kollider-hedge = import ../default.nix;

  baseImage = pkgs.dockerTools.pullImage {
      imageName = "debian";
      imageDigest = "sha256:7d8264bf731fec57d807d1918bec0a16550f52a9766f0034b40f55c5b7dc3712";
      sha256 = "sha256-PwMVlEk81ALRwDCSdb9LLdJ1zr6tn4EMxcqtlvxihnE=";
    };

  # As we place all executables in single derivation the derivation takes them
  # from it and allows us to make thin containers for each one.
  takeOnly = name: path: pkgs.runCommandNoCC "only-${name}" {} ''
    mkdir -p $out
    cp ${path} $out/${name}
  '';
  takeFolder = name: path: innerPath: pkgs.runCommandNoCC "folder-${name}" {} ''
    mkdir -p $out/${innerPath}
    cp -r ${path}/* $out/${innerPath}
  '';

  mkDockerImage = name: cnts: pkgs.dockerTools.buildImage {
    name = "${prefixName}${name}";
    fromImage = baseImage;
    tag = containerTag;
    contents = cnts;
    config = {
      # Entrypoint = [
      #   "/kollider-hedge"
      # ];
    };
  };

  kollider-hedge-container = mkDockerImage "kollider-hedge" [
    (takeOnly "kollider-hedge" "${kollider-hedge}/bin/kollider-hedge")
    (takeOnly "wait-for-it.sh" "${kollider-hedge.src}/wait-for-it.sh")
  ];
in { inherit
  kollider-hedge-container
  ;
}
